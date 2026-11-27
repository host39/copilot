//! The Anthropic adapter: the Messages API (`POST {base}/v1/messages`, named SSE events) and the
//! model list (`GET {base}/v1/models`). Every connection of the `anthropic` kind uses it, the
//! `anthropic` preset and `custom` gateways alike (Adapter Rules in `docs/design/copilot.md`).
//!
//! Every request sends `anthropic-version` (`copilot_core::ANTHROPIC_VERSION`, the current API
//! version per `docs/research/providers.md`) and the key in `x-api-key` only. A base URL that
//! already ends in `/v1` is not given a second one.

mod decode;
mod models;
mod request;

use std::collections::VecDeque;

use copilot_core::{
    ANTHROPIC_VERSION, DomainResult, LlmEvent, LlmRequest, ModelTarget, Outbound, ProviderTarget,
    RemoteModel,
};
use futures_util::stream::{self, BoxStream, StreamExt};
use reqwest::header::{ACCEPT, HeaderValue};
use reqwest::{Client, RequestBuilder};

use crate::anthropic::decode::{Decoder, Flow};
use crate::http::{api_key, client, send};
use crate::sse::{events, is_json, json_reply};

/// The header that carries the key.
const API_KEY: &str = "x-api-key";
/// The header that names the API version.
const VERSION: &str = "anthropic-version";

/// The connection's models, with the capabilities the provider reports.
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
    let body = request::body(&request)?;
    let client = client(clients, provider)?;
    let http = client
        .post(endpoint(provider.base_url.as_str(), "messages"))
        .header(ACCEPT, HeaderValue::from_static("text/event-stream"))
        .json(&body);
    let response = send(provider, headers(provider, http)?).await?;
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

/// `{base}/v1/<path>`, or `{base}/<path>` when the base URL already ends in `/v1`.
fn endpoint(base: &str, path: &str) -> String {
    if base.ends_with("/v1") {
        format!("{base}/{path}")
    } else {
        format!("{base}/v1/{path}")
    }
}

/// The API version and, when the connection has a key, `x-api-key`.
fn headers(target: &ProviderTarget, request: RequestBuilder) -> DomainResult<RequestBuilder> {
    let request = request.header(VERSION, HeaderValue::from_static(ANTHROPIC_VERSION));
    Ok(match api_key(target)? {
        Some(value) => request.header(API_KEY, value),
        None => request,
    })
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
            None => {
                state.decoder.end(&mut out);
                state.done = true;
            }
        }
        state.queue.extend(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_v1_unless_the_base_url_has_it() {
        assert_eq!(
            endpoint("https://api.anthropic.com", "messages"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            endpoint("https://gateway.example.com/v1", "models"),
            "https://gateway.example.com/v1/models"
        );
        assert_eq!(
            endpoint("http://localhost:4000/anthropic", "messages"),
            "http://localhost:4000/anthropic/v1/messages"
        );
    }
}
