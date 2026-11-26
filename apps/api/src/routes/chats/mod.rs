//! `/api/chats`: chats, messages, turns (server-sent `ChatStreamEvent`s), approvals, and read
//! state (`ChatService`, `ChatEngine`). Roadmap step P1.9 writes `POST /chats/{id}/messages`,
//! P2.A the other handlers, and P3.1 the run registry behind the streaming routes. Attachments
//! are in `routes::files`.

use copilot_contracts::{
    ChatListQuery, ChatResponse, ChatStreamEvent, ChatSummaryResponse, CreateChatRequest,
    DecisionsRequest, DetailsResponse, ErrorResponse, ListResponse, MessageResponse, PageResponse,
    ProjectResponse, SendMessageRequest, UpdateChatRequest,
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
        .routes(routes!(read))
        .routes(routes!(messages, send))
        .routes(routes!(regenerate))
        .routes(routes!(retry))
        .routes(routes!(decide))
        .routes(routes!(stream))
        .routes(routes!(stop))
        .routes(routes!(save_as_project))
        .routes(routes!(save_instructions))
}

#[utoipa::path(
    get,
    path = "/chats",
    operation_id = "chats_list",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(ChatListQuery),
    responses(
        (status = OK, description = "One page of the caller's chats, most recently updated first", body = PageResponse<ChatSummaryResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn list(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/chats",
    operation_id = "chats_create",
    tag = "chats",
    security(("bearerAuth" = [])),
    request_body = CreateChatRequest,
    responses(
        (status = CREATED, description = "The new chat", body = DetailsResponse<ChatResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn create(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    get,
    path = "/chats/{id}",
    operation_id = "chats_get",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id")),
    responses(
        (status = OK, description = "The chat", body = DetailsResponse<ChatResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn get(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    patch,
    path = "/chats/{id}",
    operation_id = "chats_update",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id")),
    request_body = UpdateChatRequest,
    responses(
        (status = OK, description = "The saved chat", body = DetailsResponse<ChatResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn update(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    delete,
    path = "/chats/{id}",
    operation_id = "chats_delete",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id")),
    responses(
        (status = NO_CONTENT, description = "Deleted with its messages and attachments"),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn delete(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/chats/{id}/read",
    operation_id = "chats_read",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id")),
    responses(
        (status = NO_CONTENT, description = "Marked as read"),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn read(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    get,
    path = "/chats/{id}/messages",
    operation_id = "chats_messages",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id")),
    responses(
        (status = OK, description = "The chat's messages, oldest first", body = ListResponse<MessageResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn messages(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/chats/{id}/messages",
    operation_id = "chats_send",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id")),
    request_body = SendMessageRequest,
    responses(
        (status = OK, description = "The turn as server-sent events", content_type = "text/event-stream", body = ChatStreamEvent),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn send(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/chats/{id}/regenerate",
    operation_id = "chats_regenerate",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id")),
    responses(
        (status = OK, description = "The new answer as server-sent events", content_type = "text/event-stream", body = ChatStreamEvent),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn regenerate(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/chats/{id}/retry",
    operation_id = "chats_retry",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id")),
    responses(
        (status = OK, description = "The retried turn as server-sent events", content_type = "text/event-stream", body = ChatStreamEvent),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn retry(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/chats/{id}/messages/{messageId}/decisions",
    operation_id = "chats_decide",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(
        ("id" = Uuid, Path, description = "The chat's id"),
        ("messageId" = Uuid, Path, description = "The id of the message that awaits approval")
    ),
    request_body = DecisionsRequest,
    responses(
        (status = OK, description = "The continued turn as server-sent events", content_type = "text/event-stream", body = ChatStreamEvent),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn decide(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    get,
    path = "/chats/{id}/stream",
    operation_id = "chats_stream",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id")),
    responses(
        (status = OK, description = "The running turn's events from the start, then live ones", content_type = "text/event-stream", body = ChatStreamEvent),
        (status = NO_CONTENT, description = "No turn is running"),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn stream(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/chats/{id}/stop",
    operation_id = "chats_stop",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id")),
    responses(
        (status = NO_CONTENT, description = "The running turn stops and keeps its partial answer"),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn stop(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/chats/{id}/save-as-project",
    operation_id = "chats_save_as_project",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id")),
    responses(
        (status = OK, description = "The new project the chat moved into", body = DetailsResponse<ProjectResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn save_as_project(Caller(_caller): Caller) -> HttpError {
    pending()
}

#[utoipa::path(
    post,
    path = "/chats/{id}/save-instructions",
    operation_id = "chats_save_instructions",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id")),
    responses(
        (status = OK, description = "The project that now has the chat's instructions", body = DetailsResponse<ProjectResponse>),
        (status = NOT_IMPLEMENTED, description = "Not available yet", body = ErrorResponse)
    )
)]
pub async fn save_instructions(Caller(_caller): Caller) -> HttpError {
    pending()
}
