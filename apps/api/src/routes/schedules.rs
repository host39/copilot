//! `/api/schedules`: prompts that run at set times (`ScheduleService`). A schedule's runs are
//! `GET /api/chats?scheduleId=`. Roadmap step P2.A writes the handlers (P3.2 the scheduler).

use copilot_contracts::{
    CreateScheduleRequest, DetailsResponse, ErrorResponse, ListResponse, ScheduleListQuery,
    SchedulePreviewRequest, SchedulePreviewResponse, ScheduleResponse, ScheduleRunResponse,
    UpdateScheduleRequest,
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
        .routes(routes!(run))
        .routes(routes!(preview))
}

#[utoipa::path(
    get,
    path = "/schedules",
    operation_id = "schedules_list",
    tag = "schedules",
    security(("bearerAuth" = [])),
    params(ScheduleListQuery),
    responses(
        (status = OK, description = "The caller's schedules, oldest first", body = ListResponse<ScheduleResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn list(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/schedules",
    operation_id = "schedules_create",
    tag = "schedules",
    security(("bearerAuth" = [])),
    request_body = CreateScheduleRequest,
    responses(
        (status = CREATED, description = "The new schedule", body = DetailsResponse<ScheduleResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn create(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    get,
    path = "/schedules/{id}",
    operation_id = "schedules_get",
    tag = "schedules",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The schedule's id")),
    responses(
        (status = OK, description = "The schedule", body = DetailsResponse<ScheduleResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn get(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    patch,
    path = "/schedules/{id}",
    operation_id = "schedules_update",
    tag = "schedules",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The schedule's id")),
    request_body = UpdateScheduleRequest,
    responses(
        (status = OK, description = "The saved schedule", body = DetailsResponse<ScheduleResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn update(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    delete,
    path = "/schedules/{id}",
    operation_id = "schedules_delete",
    tag = "schedules",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The schedule's id")),
    responses(
        (status = NO_CONTENT, description = "Deleted; the chats of its runs stay"),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn delete(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/schedules/{id}/run",
    operation_id = "schedules_run",
    tag = "schedules",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The schedule's id")),
    responses(
        (status = OK, description = "The run started; attach with `GET /chats/{id}/stream`", body = DetailsResponse<ScheduleRunResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn run(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/schedules/preview",
    operation_id = "schedules_preview",
    tag = "schedules",
    security(("bearerAuth" = [])),
    request_body = SchedulePreviewRequest,
    responses(
        (status = OK, description = "The rule in words and its next runs", body = DetailsResponse<SchedulePreviewResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn preview(Caller(_caller): Caller) -> HttpError {
    pending()
}
