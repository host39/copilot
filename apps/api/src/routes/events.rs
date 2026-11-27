//! `/api/events`: the caller's live updates as server-sent events (`UserEvent`), so open clients
//! refresh the sidebar without polling. The run registry fans them out
//! ([`RunRegistry::watch`](crate::helpers::runs::RunRegistry::watch)): the chat routes publish
//! `chat.updated`, and each run `run.started`, then `approval.pending` when it pauses for an
//! approval, and `run.finished`.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use copilot_contracts::UserEvent;
use copilot_core::shared::constants::{EVENTS_STREAM_MAX_SECS, SSE_KEEP_ALIVE_SECS};
use futures_util::StreamExt;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::Caller;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(events))
}

#[utoipa::path(
    get,
    path = "/events",
    operation_id = "events_events",
    tag = "events",
    security(("bearerAuth" = [])),
    responses(
        (status = OK, description = "The caller's `UserEvent`s: `run.started` for each chat answering now, then live events. The stream ends after `EVENTS_STREAM_MAX_SECS`, when it falls behind, or on shutdown; connect again and read the chat list afresh", content_type = "text/event-stream", body = UserEvent)
    )
)]
pub async fn events(State(state): State<AppState>, Caller(caller): Caller) -> Response {
    let events = state
        .runs
        .watch(caller.user_id)
        .into_stream()
        .map(|event| Ok::<_, Infallible>(sse(&event)))
        .take_until(state.shutdown.clone().cancelled_owned())
        .take_until(tokio::time::sleep(Duration::from_secs(
            EVENTS_STREAM_MAX_SECS,
        )));
    Sse::new(events)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(SSE_KEEP_ALIVE_SECS)))
        .into_response()
}

fn sse(event: &UserEvent) -> Event {
    Event::default()
        .json_data(event)
        .unwrap_or_else(|_| Event::default().comment("event.unserializable"))
}
