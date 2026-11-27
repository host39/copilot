//! What both wire formats share: streaming events with pauses, request ids, and timestamps.

use std::convert::Infallible;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::http::{HeaderName, HeaderValue};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::stream::{self, StreamExt};

/// Server-sent `events`, each sent after a pause of `delay`.
pub(crate) fn stream(events: Vec<Event>, delay: Duration) -> Response {
    let events = stream::iter(events).then(move |event| async move {
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        Ok::<_, Infallible>(event)
    });
    Sse::new(events).into_response()
}

/// Adds the request id header the provider's logs would carry (`x-request-id` on OpenAI,
/// `request-id` on Anthropic).
pub(crate) fn with_id(mut response: Response, header: &'static str, id: &str) -> Response {
    if let Ok(value) = HeaderValue::from_str(id) {
        response
            .headers_mut()
            .insert(HeaderName::from_static(header), value);
    }
    response
}

/// Seconds since the Unix epoch.
pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// `text` in pieces of at most `size` characters (tool arguments stream this way).
pub(crate) fn pieces(text: &str, size: usize) -> Vec<String> {
    let characters: Vec<char> = text.chars().collect();
    characters
        .chunks(size.max(1))
        .map(|piece| piece.iter().collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_text_into_pieces_on_character_boundaries() {
        assert_eq!(pieces("abcdefg", 3), vec!["abc", "def", "g"]);
        assert_eq!(pieces("été", 2), vec!["ét", "é"]);
        assert_eq!(pieces("", 3), Vec::<String>::new());
    }
}
