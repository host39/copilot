//! `/api/projects`: projects (`ProjectService`). Their files are in `routes::files`. Roadmap step
//! P2.A writes the handlers.

use copilot_contracts::{
    CreateProjectRequest, DetailsResponse, ErrorResponse, ListResponse, ProjectDetailsResponse,
    ProjectListQuery, ProjectResponse, UpdateProjectRequest,
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
}

#[utoipa::path(
    get,
    path = "/projects",
    operation_id = "projects_list",
    tag = "projects",
    security(("bearerAuth" = [])),
    params(ProjectListQuery),
    responses(
        (status = OK, description = "The caller's projects, most recently updated first", body = ListResponse<ProjectResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn list(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/projects",
    operation_id = "projects_create",
    tag = "projects",
    security(("bearerAuth" = [])),
    request_body = CreateProjectRequest,
    responses(
        (status = CREATED, description = "The new project", body = DetailsResponse<ProjectResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn create(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    get,
    path = "/projects/{id}",
    operation_id = "projects_get",
    tag = "projects",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The project's id")),
    responses(
        (status = OK, description = "The project with its capacity", body = DetailsResponse<ProjectDetailsResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn get(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    patch,
    path = "/projects/{id}",
    operation_id = "projects_update",
    tag = "projects",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The project's id")),
    request_body = UpdateProjectRequest,
    responses(
        (status = OK, description = "The saved project", body = DetailsResponse<ProjectResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn update(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    delete,
    path = "/projects/{id}",
    operation_id = "projects_delete",
    tag = "projects",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The project's id")),
    responses(
        (status = NO_CONTENT, description = "Deleted with its chats, files, and schedules"),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn delete(Caller(_caller): Caller) -> HttpError {
    pending()
}
