//! `/api/tool-servers`: MCP tool servers, their tool modes, tests, and reviews (`ToolService`).
//! Roadmap step P2.A writes the handlers.

use copilot_contracts::{
    CreateToolServerRequest, DetailsResponse, ErrorResponse, ListResponse, ToolServerResponse,
    ToolTestResponse, UpdateToolServerRequest,
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
        .routes(routes!(test))
        .routes(routes!(review))
}

#[utoipa::path(
    get,
    path = "/tool-servers",
    operation_id = "tool_servers_list",
    tag = "tool-servers",
    security(("bearerAuth" = [])),
    responses(
        (status = OK, description = "The caller's tool servers, oldest first", body = ListResponse<ToolServerResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn list(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/tool-servers",
    operation_id = "tool_servers_create",
    tag = "tool-servers",
    security(("bearerAuth" = [])),
    request_body = CreateToolServerRequest,
    responses(
        (status = CREATED, description = "The new tool server", body = DetailsResponse<ToolServerResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn create(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    get,
    path = "/tool-servers/{id}",
    operation_id = "tool_servers_get",
    tag = "tool-servers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The tool server's id")),
    responses(
        (status = OK, description = "The tool server with its tool modes", body = DetailsResponse<ToolServerResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn get(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    patch,
    path = "/tool-servers/{id}",
    operation_id = "tool_servers_update",
    tag = "tool-servers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The tool server's id")),
    request_body = UpdateToolServerRequest,
    responses(
        (status = OK, description = "The saved tool server", body = DetailsResponse<ToolServerResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn update(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    delete,
    path = "/tool-servers/{id}",
    operation_id = "tool_servers_delete",
    tag = "tool-servers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The tool server's id")),
    responses(
        (status = NO_CONTENT, description = "Deleted"),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn delete(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/tool-servers/{id}/test",
    operation_id = "tool_servers_test",
    tag = "tool-servers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The tool server's id")),
    responses(
        (status = OK, description = "Connected, listed the tools, and refreshed the cache", body = DetailsResponse<ToolTestResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn test(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/tool-servers/{id}/review",
    operation_id = "tool_servers_review",
    tag = "tool-servers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The tool server's id")),
    responses(
        (status = OK, description = "The changed tool list is accepted", body = DetailsResponse<ToolServerResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn review(Caller(_caller): Caller) -> HttpError {
    pending()
}
