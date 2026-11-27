//! Server-sent event responses, shared by the run streams and `/api/events`: a keep-alive
//! comment every `SSE_KEEP_ALIVE_SECS`, `Cache-Control: no-cache` (axum's), and
//! `X-Accel-Buffering: no`, so an nginx-based proxy forwards each event at once even where its
//! buffering cannot be turned off. A stream ends on shutdown and once its session has ended.

use std::convert::Infallible;
use std::time::Duration;

use axum::http::{HeaderName, HeaderValue};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use copilot_core::shared::constants::SSE_KEEP_ALIVE_SECS;
use futures_util::{Stream, StreamExt};
use serde::Serialize;

use crate::config::AppState;
use crate::helpers::auth::Session;

const X_ACCEL_BUFFERING: HeaderName = HeaderName::from_static("x-accel-buffering");

/// `events` as a server-sent event response for the holder of `session`.
pub fn respond<S>(state: &AppState, session: Session, events: S) -> Response
where
    S: Stream<Item = Event> + Send + 'static,
{
    let events = events
        .map(Ok::<_, Infallible>)
        .take_until(state.shutdown.clone().cancelled_owned())
        .take_until(state.identity.clone().ended(session));
    let mut response = Sse::new(events)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(SSE_KEEP_ALIVE_SECS)))
        .into_response();
    response
        .headers_mut()
        .insert(X_ACCEL_BUFFERING, HeaderValue::from_static("no"));
    response
}

/// `value` as one event's JSON data, with `id` when given. A value that cannot be serialized
/// becomes a comment (`event.unserializable`).
pub fn event<T: Serialize>(value: &T, id: Option<u64>) -> Event {
    let event = match id {
        Some(id) => Event::default().id(id.to_string()),
        None => Event::default(),
    };
    event
        .json_data(value)
        .unwrap_or_else(|_| Event::default().comment("event.unserializable"))
}
