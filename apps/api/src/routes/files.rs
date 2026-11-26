//! Files: project files, chat attachments, and their bytes (`FileService`). Uploads are
//! `multipart/form-data` with one part named `file` (`UPLOAD_FIELD`); the content route sends the
//! safety headers of "Files" in `docs/design/copilot.md`. Roadmap step P2.A writes the handlers.

use copilot_contracts::{
    DetailsResponse, ErrorResponse, FileResponse, ListResponse, MoveFileRequest, UpdateFileRequest,
    UploadFileForm,
};
use copilot_middleware::HttpError;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::Caller;
use crate::routes::pending;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(upload_to_project, list))
        .routes(routes!(upload_to_chat))
        .routes(routes!(update, delete))
        .routes(routes!(move_to))
        .routes(routes!(content))
}

#[utoipa::path(
    post,
    path = "/projects/{id}/files",
    operation_id = "files_upload_to_project",
    tag = "files",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The project's id")),
    request_body(content = UploadFileForm, content_type = "multipart/form-data"),
    responses(
        (status = CREATED, description = "The stored file", body = DetailsResponse<FileResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn upload_to_project(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    get,
    path = "/projects/{id}/files",
    operation_id = "files_list",
    tag = "files",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The project's id")),
    responses(
        (status = OK, description = "The project's files, oldest first", body = ListResponse<FileResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn list(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/chats/{id}/files",
    operation_id = "files_upload_to_chat",
    tag = "files",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id")),
    request_body(content = UploadFileForm, content_type = "multipart/form-data"),
    responses(
        (status = CREATED, description = "The stored attachment", body = DetailsResponse<FileResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn upload_to_chat(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    patch,
    path = "/files/{id}",
    operation_id = "files_update",
    tag = "files",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The file's id")),
    request_body = UpdateFileRequest,
    responses(
        (status = OK, description = "The saved file", body = DetailsResponse<FileResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn update(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    delete,
    path = "/files/{id}",
    operation_id = "files_delete",
    tag = "files",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The file's id")),
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
    path = "/files/{id}/move",
    operation_id = "files_move_to",
    tag = "files",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The file's id")),
    request_body = MoveFileRequest,
    responses(
        (status = OK, description = "The file in its new project", body = DetailsResponse<FileResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn move_to(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    get,
    path = "/files/{id}/content",
    operation_id = "files_content",
    tag = "files",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The file's id")),
    responses(
        (status = OK, description = "The file's bytes, as an attachment", content_type = "application/octet-stream"),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn content(Caller(_caller): Caller) -> HttpError {
    pending()
}
