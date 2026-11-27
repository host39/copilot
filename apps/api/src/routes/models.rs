//! `/api/models`: the model picker and per-model settings (`ModelService`).

use axum::extract::State;
use copilot_contracts::{
    CapabilityLock, DetailsResponse, ErrorResponse, ListResponse, ModelResponse, UpdateModelRequest,
};
use copilot_core::{ModelId, ModelQuery, ModelUpdate, ModelView};
use copilot_middleware::{HttpError, JsonBody};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::Caller;
use crate::routes::path::Id;
use crate::wire::convert::{ToCore, ToDto};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list))
        .routes(routes!(update))
}

/// A model with its connection, effective and detected capabilities, overrides, and locks.
pub fn response(view: &ModelView) -> ModelResponse {
    let model = &view.model;
    ModelResponse {
        id: model.id().uuid(),
        provider_id: model.provider_id().uuid(),
        provider_name: view.provider_name.clone(),
        kind: view.kind.to_dto(),
        preset: view.preset.to_dto(),
        remote_id: model.remote_id().to_owned(),
        display_name: model.display_name().to_owned(),
        enabled: model.enabled(),
        available: model.available(),
        context_window: model.context_window(),
        max_output_tokens: model.max_output_tokens(),
        capabilities: view.capabilities.to_dto(),
        detected: model.capabilities().to_dto(),
        overrides: model.overrides().to_dto(),
        locks: view
            .locks
            .iter()
            .map(|lock| CapabilityLock {
                capability: lock.capability.to_dto(),
                reason: lock.reason.to_owned(),
            })
            .collect(),
        reasoning_off_with_tools: view.reasoning_off_with_tools,
        created_at: model.created_at(),
        updated_at: model.updated_at(),
    }
}

pub fn list_response(views: &[ModelView]) -> ListResponse<ModelResponse> {
    ListResponse {
        total: views.len() as u64,
        items: views.iter().map(response).collect(),
    }
}

#[utoipa::path(
    get,
    path = "/models",
    operation_id = "models_list",
    tag = "models",
    security(("bearerAuth" = [])),
    responses(
        (status = OK, description = "Enabled and available models, for the picker", body = ListResponse<ModelResponse>)
    )
)]
pub async fn list(
    State(state): State<AppState>,
    Caller(caller): Caller,
) -> Result<axum::Json<ListResponse<ModelResponse>>, HttpError> {
    let views = state.models.list(&caller, ModelQuery::picker()).await?;
    Ok(axum::Json(list_response(&views)))
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
        (status = BAD_REQUEST, description = "A field is not valid", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such model", body = ErrorResponse)
    )
)]
pub async fn update(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
    JsonBody(body): JsonBody<UpdateModelRequest>,
) -> Result<axum::Json<DetailsResponse<ModelResponse>>, HttpError> {
    let view = state
        .models
        .update(
            &caller,
            ModelId::new(id),
            ModelUpdate {
                enabled: body.enabled,
                display_name: body.display_name,
                capability_overrides: body.capability_overrides.map(ToCore::to_core),
                context_window: body.context_window,
                max_output_tokens: body.max_output_tokens,
            },
        )
        .await?;
    Ok(axum::Json(DetailsResponse {
        data: response(&view),
    }))
}
