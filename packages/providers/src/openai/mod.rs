//! The OpenAI-compatible adapter: Chat Completions (`POST {base}/chat/completions`, data-only
//! SSE) and the model list (`GET {base}/models`). Every preset of the `openai` kind uses it; the
//! dialect sets the per-preset details (`copilot_core::Dialect`, and Adapter Rules in
//! `docs/design/copilot.md`).

mod calls;
mod decode;
mod models;
mod replay;
mod request;

use std::collections::VecDeque;

use copilot_core::{
    DomainError, DomainResult, LlmEvent, LlmRequest, ModelTarget, Outbound, ProviderTarget,
    RemoteModel,
};
use futures_util::stream::{self, BoxStream, StreamExt};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use reqwest::{Client, Response};
use serde_json::Value;

use crate::http::{bad_response, bearer, body, client, send};
use crate::openai::decode::{Decoder, Flow, failed};
use crate::sse::events;

/// The connection's models, with what the provider reports about each.
pub(crate) async fn models(
    clients: &Outbound<Client>,
    target: &ProviderTarget,
) -> DomainResult<Vec<RemoteModel>> {
    models::list(clients, target).await
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
    if is_json(&response) {
        return Err(json_reply(provider, response).await);
    }
    let state = State {
        events: events(provider.provider_id, response.bytes_stream()),
        decoder: Decoder::new(provider.provider_id, target.dialect),
        queue: VecDeque::new(),
        done: false,
    };
    Ok(stream::unfold(state, next).boxed())
}

/// Whether a success response is a JSON document rather than an event stream: some servers
/// answer an error (or ignore `stream: true`) that way.
fn is_json(response: &Response) -> bool {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
        })
}

/// The error for a JSON reply to a streamed request: the provider's error, or a reply Copilot
/// cannot read. The body is never echoed.
async fn json_reply(provider: &ProviderTarget, response: Response) -> DomainError {
    let bytes = match body(provider, response).await {
        Ok(bytes) => bytes,
        Err(error) => return error,
    };
    let reply: Option<Value> = serde_json::from_slice(&bytes).ok();
    let error_reply = reply.as_ref().is_some_and(|reply| {
        reply.get("error").is_some() || reply.get("object").and_then(Value::as_str) == Some("error")
    });
    tracing::warn!(
        provider_id = %provider.provider_id,
        error_reply,
        "provider.stream.not-streamed"
    );
    if error_reply {
        failed()
    } else {
        bad_response()
    }
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
                    state.queue.push_back(Err(state.decoder.error_event()));
                    state.done = true;
                }
                Some(_) => {}
            },
            Some(Err(error)) => {
                state.queue.push_back(Err(error));
                state.done = true;
            }
            None => {
                let mut out = Vec::new();
                state.decoder.end(&mut out);
                state.queue.extend(out);
                state.done = true;
            }
        }
    }
}
