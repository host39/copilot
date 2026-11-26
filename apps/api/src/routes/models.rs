//! `/api/models`: the model picker and per-model settings (`ModelService`). Roadmap step P2.A
//! writes the handlers.

use copilot_contracts::{
    DetailsResponse, ErrorResponse, ListResponse, ModelResponse, UpdateModelRequest,
};
use copilot_middleware::HttpError;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::Caller;
use crate::routes::pending;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list))
        .routes(routes!(update))
}

#[utoipa::path(
    get,
    path = "/models",
    operation_id = "models_list",
    tag = "models",
    security(("bearerAuth" = [])),
    responses(
        (status = OK, description = "Enabled and available models, for the picker", body = ListResponse<ModelResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn list(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    patch,
    path = "/models/{id}",
    operation_id = "models_update",
    tag = "models",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The model's id")),
    request_body = UpdateModelRequest,
    responses(
        (status = OK, description = "The saved model", body = DetailsResponse<ModelResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn update(Caller(_caller): Caller) -> HttpError {
    pending()
}
