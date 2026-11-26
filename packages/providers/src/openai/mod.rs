//! The OpenAI-compatible adapter: Chat Completions (`POST {base}/chat/completions`, data-only
//! SSE) and the model list (`GET {base}/models`). Every preset of the `openai` kind uses it; the
//! dialect sets the per-preset details (`copilot_core::Dialect`).

mod decode;
mod request;

use std::collections::VecDeque;

use copilot_core::{
    DomainResult, LlmEvent, LlmRequest, ModelTarget, Outbound, ProviderTarget, RemoteModel,
};
use futures_util::stream::{self, BoxStream, StreamExt};
use reqwest::Client;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderValue};
use serde::Deserialize;

use crate::http::{bad_response, bearer, body, client, send};
use crate::openai::decode::{Decoder, Flow};
use crate::sse::events;

/// `GET /models`.
#[derive(Debug, Deserialize)]
struct ModelList {
    data: Vec<ListedModel>,
}

#[derive(Debug, Deserialize)]
struct ListedModel {
    id: String,
}

/// The connection's models. The list carries no names or capabilities, so core applies the
/// preset defaults (`Capabilities::defaults`).
pub(crate) async fn models(
    clients: &Outbound<Client>,
    target: &ProviderTarget,
) -> DomainResult<Vec<RemoteModel>> {
    let client = client(clients, target)?;
    let mut request = client.get(format!("{}/models", target.base_url.as_str()));
    if let Some(value) = bearer(target)? {
        request = request.header(AUTHORIZATION, value);
    }
    let response = send(target, request).await?;
    let bytes = body(target, response).await?;
    let list: ModelList = serde_json::from_slice(&bytes).map_err(|error| {
        tracing::warn!(provider_id = %target.provider_id, error = %error, "provider.models.invalid");
        bad_response()
    })?;
    Ok(list
        .data
        .into_iter()
        .map(|model| RemoteModel {
            display_name: model.id.clone(),
            remote_id: model.id,
            ..RemoteModel::default()
        })
        .collect())
}

/// Streams one answer. The request body is built before connecting, so a request the adapter
/// cannot send fails without reaching the provider.
pub(crate) async fn stream(
    clients: &Outbound<Client>,
    target: &ModelTarget,
    request: LlmRequest,
) -> DomainResult<BoxStream<'static, DomainResult<LlmEvent>>> {
    let provider = &target.provider;
    let body = request::body(target, &request)?;
    let client = client(clients, provider)?;
    let mut http = client
        .post(format!("{}/chat/completions", provider.base_url.as_str()))
        .header(ACCEPT, HeaderValue::from_static("text/event-stream"))
        .json(&body);
    if let Some(value) = bearer(provider)? {
        http = http.header(AUTHORIZATION, value);
    }
    let response = send(provider, http).await?;
    let state = State {
        events: events(provider.provider_id, response.bytes_stream()),
        decoder: Decoder::new(provider.provider_id),
        queue: VecDeque::new(),
        done: false,
    };
    Ok(stream::unfold(state, next).boxed())
}

/// The decoding state of one answer.
struct State {
    events: BoxStream<'static, DomainResult<sse_stream::Sse>>,
    decoder: Decoder,
    /// Decoded events not yet handed out.
    queue: VecDeque<DomainResult<LlmEvent>>,
    /// No more SSE events are read.
    done: bool,
}

async fn next(mut state: State) -> Option<(DomainResult<LlmEvent>, State)> {
    loop {
        if let Some(item) = state.queue.pop_front() {
            return Some((item, state));
        }
        if state.done {
            return None;
        }
        match state.events.next().await {
            Some(Ok(event)) => match event.event.as_deref() {
                // Chat Completions sends data-only events; data-less blocks carry nothing.
                None | Some("message") => {
                    let Some(data) = event.data else {
                        continue;
                    };
                    let mut out = Vec::new();
                    state.done = state.decoder.feed(&data, &mut out) == Flow::Done;
                    state.queue.extend(out);
                }
                Some("error") => {
                    state.queue.push_back(Err(state.decoder.failed()));
                    state.done = true;
                }
                Some(_) => {}
            },
            Some(Err(error)) => {
                state.queue.push_back(Err(error));
                state.done = true;
            }
            None => {
                state.queue.extend(state.decoder.end().map(Ok));
                state.done = true;
            }
        }
    }
}
