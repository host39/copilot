//! `/api/settings`: the caller's personal instructions (`SettingsService`). Roadmap step P2.A
//! writes the handlers.

use copilot_contracts::{DetailsResponse, ErrorResponse, SettingsResponse, UpdateSettingsRequest};
use copilot_middleware::HttpError;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::Caller;
use crate::routes::pending;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get, update))
}

#[utoipa::path(
    get,
    path = "/settings",
    operation_id = "settings_get",
    tag = "settings",
    security(("bearerAuth" = [])),
    responses(
        (status = OK, description = "The caller's settings", body = DetailsResponse<SettingsResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn get(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    patch,
    path = "/settings",
    operation_id = "settings_update",
    tag = "settings",
    security(("bearerAuth" = [])),
    request_body = UpdateSettingsRequest,
    responses(
        (status = OK, description = "The saved settings", body = DetailsResponse<SettingsResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn update(Caller(_caller): Caller) -> HttpError {
    pending()
}
