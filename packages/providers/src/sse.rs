//! Server-sent events from a provider response, each event capped at
//! [`RESPONSE_MAX_BYTES`](crate::http::RESPONSE_MAX_BYTES) before the parser buffers it.

use bytes::Bytes;
use copilot_core::{CapError, DomainError, DomainResult, ProviderId, cap_events};
use futures_util::stream::{BoxStream, Stream, StreamExt};
use sse_stream::{Error as SseError, Sse, SseByteStream};

use crate::http::{PROVIDER_BAD_RESPONSE, RESPONSE_MAX_BYTES};

/// The code of an answer whose connection broke or went silent mid-stream.
pub(crate) const PROVIDER_STREAM_FAILED: &str = "PROVIDER_STREAM_FAILED";

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
        .map(move |item| item.map_err(|error| failed(provider, &error)))
        .boxed()
}

fn failed(provider: ProviderId, error: &SseError) -> DomainError {
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
