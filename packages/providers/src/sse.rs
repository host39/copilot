//! Server-sent events from a provider response, each event capped at
//! [`RESPONSE_MAX_BYTES`](crate::http::RESPONSE_MAX_BYTES) before the parser buffers it, and
//! what every streamed answer shares: the error of a provider that ended it with an error, and
//! the check for a JSON reply to a streamed request.

use bytes::Bytes;
use copilot_core::{CapError, DomainError, DomainResult, ProviderId, ProviderTarget, cap_events};
use futures_util::stream::{BoxStream, Stream, StreamExt};
use reqwest::Response;
use reqwest::header::CONTENT_TYPE;
use serde_json::Value;
use sse_stream::{Error as SseError, Sse, SseByteStream};

use crate::http::{PROVIDER_BAD_RESPONSE, RESPONSE_MAX_BYTES, bad_response, body};

/// The code of an answer whose connection broke or went silent mid-stream.
pub(crate) const PROVIDER_STREAM_FAILED: &str = "PROVIDER_STREAM_FAILED";
/// The code of an answer the provider ended with an error (an error chunk or event).
pub(crate) const PROVIDER_ERROR: &str = "PROVIDER_ERROR";

/// The provider ended the answer with an error (an error chunk, an `error` event, or a JSON
/// error reply to a streamed request). Its text is not echoed.
pub(crate) fn failed() -> DomainError {
    DomainError::upstream(
        "The provider ended the answer with an error.",
        PROVIDER_ERROR,
    )
}

/// Whether a success response is a JSON document rather than an event stream: some servers
/// answer an error (or ignore `stream: true`) that way.
pub(crate) fn is_json(response: &Response) -> bool {
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

/// The error for a JSON reply to a streamed request: the provider's error (an `error` field,
/// vLLM's `object: "error"`, or Anthropic's `type: "error"`), or a reply Copilot cannot read.
/// The body is never echoed.
pub(crate) async fn json_reply(provider: &ProviderTarget, response: Response) -> DomainError {
    let bytes = match body(provider, response).await {
        Ok(bytes) => bytes,
        Err(error) => return error,
    };
    let reply: Option<Value> = serde_json::from_slice(&bytes).ok();
    let error_reply = reply.as_ref().is_some_and(|reply| {
        reply.get("error").is_some()
            || reply.get("object").and_then(Value::as_str) == Some("error")
            || reply.get("type").and_then(Value::as_str) == Some("error")
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

/// Decodes `bytes` (LF, CRLF, or CR line endings; comments skipped; a cut-off last event
/// dropped). The first failure is the last item: a broken connection, an event over the cap, or
/// text that is not UTF-8.
pub(crate) fn events<S>(provider: ProviderId, bytes: S) -> BoxStream<'static, DomainResult<Sse>>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
{
    // Errors drop their URL first, so neither the log nor a client ever sees it.
    let bytes = bytes.map(|chunk| chunk.map_err(reqwest::Error::without_url));
    SseByteStream::new(cap_events(bytes, RESPONSE_MAX_BYTES))
        .map(move |item| item.map_err(|error| broken(provider, &error)))
        .boxed()
}

fn broken(provider: ProviderId, error: &SseError) -> DomainError {
    let cap = match error {
        SseError::Body(source) => source.downcast_ref::<CapError<reqwest::Error>>(),
        _ => None,
    };
    match cap {
        Some(CapError::TooLarge { limit }) => {
            tracing::warn!(provider_id = %provider, limit, "provider.stream.too-large");
            DomainError::upstream(
                "The provider sent a reply over the size limit.",
                PROVIDER_BAD_RESPONSE,
            )
        }
        Some(CapError::Stream(source)) => {
            tracing::warn!(provider_id = %provider, error = %source, "provider.stream.failed");
            if source.is_timeout() {
                DomainError::upstream(
                    "The provider stopped sending before the answer was complete.",
                    PROVIDER_STREAM_FAILED,
                )
            } else {
                DomainError::upstream(
                    "The connection to the provider was lost.",
                    PROVIDER_STREAM_FAILED,
                )
            }
        }
        None => {
            tracing::warn!(provider_id = %provider, error = %error, "provider.stream.invalid");
            DomainError::upstream(
                "The provider sent a reply Copilot cannot read.",
                PROVIDER_BAD_RESPONSE,
            )
        }
    }
}
