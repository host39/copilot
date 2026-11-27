//! The Responses API adapter: `POST {base}/responses` with `store: false` and typed SSE events,
//! for `openai` connections whose API is Responses (`copilot_core::OpenAiApi`; the `openai`
//! preset by default, the `custom` preset by choice). The model list stays Chat Completions'
//! `GET {base}/models` (`crate::openai`), and so do the key (`Authorization: Bearer`) and the
//! base URL. See `docs/research/providers.md` (OpenAI Responses) and Adapter Rules in
//! `docs/design/copilot.md`.

mod decode;
mod replay;
mod request;

use std::collections::VecDeque;

use copilot_core::{DomainResult, LlmEvent, LlmRequest, ModelTarget, Outbound};
use futures_util::stream::{self, BoxStream, StreamExt};
use reqwest::Client;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderValue};

use crate::http::{bearer, client, send};
use crate::responses::decode::{Decoder, Flow};
use crate::sse::{events, is_json, json_reply};

/// Streams one answer. The request body is built before connecting, so a request the adapter
/// cannot send fails without reaching the provider.
pub(crate) async fn stream(
    clients: &Outbound<Client>,
    target: &ModelTarget,
    request: LlmRequest,
) -> DomainResult<BoxStream<'static, DomainResult<LlmEvent>>> {
    let provider = &target.provider;
    let body = request::body(&request)?;
    let client = client(clients, provider)?;
    let mut http = client
        .post(format!("{}/responses", provider.base_url.as_str()))
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
        let mut out = Vec::new();
        match state.events.next().await {
            Some(Ok(event)) => {
                let name = event.event.as_deref();
                let data = event.data.as_deref().unwrap_or_default();
                state.done = state.decoder.feed(name, data, &mut out) == Flow::Done;
            }
            Some(Err(error)) => {
                out.push(Err(error));
                state.done = true;
            }
            // A body that ends without a terminal event: the engine saves the answer as ended
            // early.
            None => state.done = true,
        }
        state.queue.extend(out);
    }
}
