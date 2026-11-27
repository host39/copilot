//! `/api/events`: the caller's live updates as server-sent events (`UserEvent`), so open clients
//! refresh the sidebar without polling. The run registry fans them out
//! ([`RunRegistry::watch`](crate::helpers::runs::RunRegistry::watch)): the chat routes publish
//! `chat.updated`, and each run `run.started`, then `approval.pending` when it pauses for an
//! approval, and `run.finished`.

use std::time::Duration;

use axum::extract::State;
use axum::response::Response;
use copilot_contracts::UserEvent;
use copilot_core::shared::constants::EVENTS_STREAM_MAX_SECS;
use futures_util::StreamExt;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::{Caller, Session};
use crate::routes::sse;

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
pub async fn events(
    State(state): State<AppState>,
    Caller(caller): Caller,
    session: Session,
) -> Response {
    let events = state
        .runs
        .watch(caller.user_id)
        .into_stream()
        .map(|event| sse::event(&event, None))
        .take_until(tokio::time::sleep(Duration::from_secs(
            EVENTS_STREAM_MAX_SECS,
        )));
    sse::respond(&state, session, events)
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use copilot_core::shared::constants::SESSION_RECHECK_SECS;
    use tokio::time::Instant;
    use tower::ServiceExt;

    use crate::helpers::testing::{app, register, request, send};

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_stream_ends_once_its_session_has_ended() {
        let app = app().await;
        let token = register(&app, "alice").await;
        let response = app
            .clone()
            .oneshot(request(Method::GET, "/api/events", Some(&token), None))
            .await
            .expect("infallible router");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        assert_eq!(response.headers()["cache-control"], "no-cache");
        assert_eq!(
            response.headers()["x-accel-buffering"],
            "no",
            "nginx-based proxies pass each event on at once"
        );

        let signed_out = send(
            app.clone(),
            request(Method::POST, "/api/auth/logout", Some(&token), None),
        )
        .await;
        assert!(signed_out.status.is_success(), "{}", signed_out.body);
        let started = Instant::now();
        let mut body = response.into_body().into_data_stream();
        while body.next().await.is_some() {}
        let took = started.elapsed();
        assert!(
            took <= Duration::from_secs(2 * SESSION_RECHECK_SECS),
            "ended at the next check, not after {EVENTS_STREAM_MAX_SECS} seconds: {took:?}"
        );
    }
}
