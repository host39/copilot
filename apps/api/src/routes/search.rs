//! `/api/search`: chat titles and message text (`ChatService::search`). Roadmap step P2.A writes
//! the handler.

use copilot_contracts::{ErrorResponse, ListResponse, SearchHitResponse, SearchQuery};
use copilot_middleware::HttpError;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::Caller;
use crate::routes::pending;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(search))
}

#[utoipa::path(
    get,
    path = "/search",
    operation_id = "search_search",
    tag = "search",
    security(("bearerAuth" = [])),
    params(SearchQuery),
    responses(
        (status = OK, description = "Matching chats and messages, best first", body = ListResponse<SearchHitResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn search(Caller(_caller): Caller) -> HttpError {
    pending()
}
