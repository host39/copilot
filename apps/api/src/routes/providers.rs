//! `/api/providers`: provider connections, their models, and refreshes (`ProviderService`,
//! `ModelService`). The walking skeleton (P1.9) writes `list`, `create`, `refresh`, and
//! `models`; roadmap step P2.A writes the rest.

use axum::extract::State;
use axum::http::StatusCode;
use copilot_contracts::{
    CreateProviderRequest, DetailsResponse, ErrorResponse, ListResponse, ModelResponse,
    ProviderDetailsResponse, ProviderResponse, RefreshResponse, UpdateProviderRequest,
};
use copilot_core::{KeyState, ModelQuery, NewProvider, Provider, ProviderId};
use copilot_middleware::{HttpError, JsonBody};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::Caller;
use crate::routes::convert::{ToCore, ToDto};
use crate::routes::models;
use crate::routes::path::Id;
use crate::routes::pending;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(get, update, delete))
        .routes(routes!(refresh))
        .routes(routes!(models))
}

/// A connection as the API shows it. `key` comes from the caller: lists report `set` for any
/// stored key without opening it.
pub fn response(provider: &Provider, key: KeyState) -> ProviderResponse {
    ProviderResponse {
        id: provider.id().uuid(),
        kind: provider.kind().to_dto(),
        preset: provider.preset().to_dto(),
        name: provider.name().to_owned(),
        base_url: provider.base_url().as_str().to_owned(),
        enabled: provider.enabled(),
        key: key.to_dto(),
        created_at: provider.created_at(),
        updated_at: provider.updated_at(),
    }
}

/// `set` when a key is stored, without opening it.
fn stored_key(provider: &Provider) -> KeyState {
    if provider.has_key() {
        KeyState::Set
    } else {
        KeyState::Missing
    }
}

#[utoipa::path(
    get,
    path = "/providers",
    operation_id = "providers_list",
    tag = "providers",
    security(("bearerAuth" = [])),
    responses(
        (status = OK, description = "The caller's connections, oldest first", body = ListResponse<ProviderResponse>)
    )
)]
pub async fn list(
    State(state): State<AppState>,
    Caller(caller): Caller,
) -> Result<axum::Json<ListResponse<ProviderResponse>>, HttpError> {
    let providers = state.providers.list(&caller).await?;
    let items: Vec<ProviderResponse> = providers
        .iter()
        .map(|provider| response(provider, stored_key(provider)))
        .collect();
    Ok(axum::Json(ListResponse {
        total: items.len() as u64,
        items,
    }))
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
        (status = BAD_REQUEST, description = "A field is not valid", body = ErrorResponse)
    )
)]
pub async fn create(
    State(state): State<AppState>,
    Caller(caller): Caller,
    JsonBody(body): JsonBody<CreateProviderRequest>,
) -> Result<(StatusCode, axum::Json<DetailsResponse<ProviderResponse>>), HttpError> {
    let provider = state
        .providers
        .create(
            &caller,
            NewProvider {
                kind: body.kind.to_core(),
                preset: body.preset.to_core(),
                name: body.name,
                base_url: body.base_url,
                api_key: body.api_key,
            },
        )
        .await?;
    let data = response(&provider, stored_key(&provider));
    Ok((StatusCode::CREATED, axum::Json(DetailsResponse { data })))
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
        (status = NOT_FOUND, description = "The caller has no such connection", body = ErrorResponse),
        (status = BAD_GATEWAY, description = "The provider could not list its models", body = ErrorResponse)
    )
)]
pub async fn refresh(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<axum::Json<DetailsResponse<RefreshResponse>>, HttpError> {
    let report = state
        .providers
        .refresh(&caller, ProviderId::new(id))
        .await?;
    Ok(axum::Json(DetailsResponse {
        data: RefreshResponse {
            added: report.added,
            updated: report.updated,
            unavailable: report.unavailable,
            enabled: report.enabled,
        },
    }))
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
        (status = NOT_FOUND, description = "The caller has no such connection", body = ErrorResponse)
    )
)]
pub async fn models(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<axum::Json<ListResponse<ModelResponse>>, HttpError> {
    let views = state
        .models
        .list(&caller, ModelQuery::provider(ProviderId::new(id)))
        .await?;
    Ok(axum::Json(models::list_response(&views)))
}
