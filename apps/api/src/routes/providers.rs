//! `/api/providers`: provider connections, their models, and refreshes (`ProviderService`,
//! `ModelService`). Roadmap step P2.A writes the handlers.

use copilot_contracts::{
    CreateProviderRequest, DetailsResponse, ErrorResponse, ListResponse, ModelResponse,
    ProviderDetailsResponse, ProviderResponse, RefreshResponse, UpdateProviderRequest,
};
use copilot_middleware::HttpError;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::Caller;
use crate::routes::pending;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(get, update, delete))
        .routes(routes!(refresh))
        .routes(routes!(models))
}

#[utoipa::path(
    get,
    path = "/providers",
    operation_id = "providers_list",
    tag = "providers",
    security(("bearerAuth" = [])),
    responses(
        (status = OK, description = "The caller's connections, oldest first", body = ListResponse<ProviderResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn list(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/providers",
    operation_id = "providers_create",
    tag = "providers",
    security(("bearerAuth" = [])),
    request_body = CreateProviderRequest,
    responses(
        (status = CREATED, description = "The new connection", body = DetailsResponse<ProviderResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn create(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    get,
    path = "/providers/{id}",
    operation_id = "providers_get",
    tag = "providers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The connection's id")),
    responses(
        (status = OK, description = "The connection with its usage counts", body = DetailsResponse<ProviderDetailsResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn get(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    patch,
    path = "/providers/{id}",
    operation_id = "providers_update",
    tag = "providers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The connection's id")),
    request_body = UpdateProviderRequest,
    responses(
        (status = OK, description = "The saved connection", body = DetailsResponse<ProviderResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn update(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    delete,
    path = "/providers/{id}",
    operation_id = "providers_delete",
    tag = "providers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The connection's id")),
    responses(
        (status = NO_CONTENT, description = "Deleted with its models"),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn delete(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/providers/{id}/refresh",
    operation_id = "providers_refresh",
    tag = "providers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The connection's id")),
    responses(
        (status = OK, description = "What the refresh changed", body = DetailsResponse<RefreshResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn refresh(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    get,
    path = "/providers/{id}/models",
    operation_id = "providers_models",
    tag = "providers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The connection's id")),
    responses(
        (status = OK, description = "Every known model of the connection", body = ListResponse<ModelResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn models(Caller(_caller): Caller) -> HttpError {
    pending()
}
