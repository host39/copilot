//! `/api/events`: the caller's live updates as server-sent events (`UserEvent`), so open clients
//! refresh the sidebar without polling. Roadmap step P3.1 writes the handler.

use copilot_contracts::{ErrorResponse, UserEvent};
use copilot_middleware::HttpError;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::Caller;
use crate::routes::pending;

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
        (status = OK, description = "A stream of `UserEvent`s", content_type = "text/event-stream", body = UserEvent),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn events(Caller(_caller): Caller) -> HttpError {
    pending()
}
