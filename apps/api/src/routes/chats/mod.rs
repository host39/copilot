//! `/api/chats`: chats, messages, turns (server-sent `ChatStreamEvent`s), approvals, and read
//! state (`ChatService`, `ChatEngine`, `RunRegistry`). The walking skeleton (P1.9) writes
//! `create`, `get`, `messages`, and `send`; roadmap step P2.A writes the other non-streaming
//! handlers, and P3.1 the other streaming routes, stop, and reattaching. Attachments are in
//! `routes::files`.

pub mod serializer;
#[cfg(test)]
mod tests;

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use copilot_contracts::{
    ChatListQuery, ChatResponse, ChatStreamEvent, ChatSummaryResponse, CreateChatRequest,
    DecisionsRequest, DetailsResponse, ErrorResponse, ListResponse, MessageResponse, PageResponse,
    ProjectResponse, SendMessageRequest, UpdateChatRequest,
};
use copilot_core::shared::constants::SSE_KEEP_ALIVE_SECS;
use copilot_core::{
    ChatId, FileId, MessageId, ModelId, NewChat, PreparedTurn, ProjectId, ServerOverride,
    ToolServerId, TurnEvent, TurnInput, TurnSink,
};
use copilot_middleware::{HttpError, JsonBody};
use futures_util::StreamExt;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::Caller;
use crate::helpers::runs::Reservation;
use crate::routes::convert::ToCore;
use crate::routes::path::Id;
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

/// Starts the prepared turn in its reserved slot and answers with its events. The run goes on
/// when the client disconnects; it stops on shutdown. The slot is released when the run ends.
fn run(state: &AppState, reservation: Reservation, turn: PreparedTurn) -> Response {
    let events = reservation.subscribe().into_stream();
    let engine = Arc::clone(&state.engine);
    let chat = turn.chat.id();
    tokio::spawn(async move {
        let sink = reservation.sink();
        let task = tokio::spawn({
            let sink: Arc<dyn TurnSink> = sink.clone();
            let cancel = reservation.cancel();
            async move { engine.run(turn, sink, cancel).await }
        });
        match task.await {
            Ok(Ok(outcome)) => {
                tracing::info!(chat_id = %chat, status = %outcome.status, "api.chats.run.finished");
            }
            Ok(Err(error)) => {
                tracing::error!(chat_id = %chat, error = %error, "api.chats.run.failed");
            }
            Err(error) => {
                tracing::error!(chat_id = %chat, error = %error, "api.chats.run.panicked");
            }
        }
        // A run that panicked never sent `done`: end its streams.
        if !sink.is_done() {
            sink.emit(TurnEvent::Error {
                message: "Something went wrong. Please try again.".to_owned(),
                code: "INTERNAL_ERROR".to_owned(),
            });
            sink.emit(TurnEvent::Done);
        }
        drop(reservation);
    });
    let events = events
        .map(|event| Ok::<_, Infallible>(sse(&event)))
        .take_until(state.shutdown.clone().cancelled_owned());
    Sse::new(events)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(SSE_KEEP_ALIVE_SECS)))
        .into_response()
}

/// One event as `data: <json>`.
fn sse(event: &ChatStreamEvent) -> Event {
    Event::default()
        .json_data(event)
        .unwrap_or_else(|_| Event::default().comment("event.unserializable"))
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
        (status = BAD_REQUEST, description = "A field is not valid", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such project, model, or tool server", body = ErrorResponse)
    )
)]
pub async fn create(
    State(state): State<AppState>,
    Caller(caller): Caller,
    JsonBody(body): JsonBody<CreateChatRequest>,
) -> Result<(StatusCode, axum::Json<DetailsResponse<ChatResponse>>), HttpError> {
    let input = NewChat {
        project_id: body.project_id.map(ProjectId::new),
        model_id: body.model_id.map(ModelId::new),
        instructions: body.instructions.unwrap_or_default(),
        reasoning: body.reasoning.map(ToCore::to_core),
        builtin_tools: body
            .builtin_tools
            .map(|tools| tools.into_iter().map(ToCore::to_core).collect()),
        tool_overrides: body
            .tool_overrides
            .unwrap_or_default()
            .into_iter()
            .map(|row| ServerOverride {
                server_id: ToolServerId::new(row.server_id),
                enabled: row.enabled,
            })
            .collect(),
        voice_replies: body.voice_replies.unwrap_or(false),
    };
    let chat = state.chats.create(&caller, input).await?;
    let data = serializer::chat(&chat);
    Ok((StatusCode::CREATED, axum::Json(DetailsResponse { data })))
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
        (status = NOT_FOUND, description = "The caller has no such chat", body = ErrorResponse)
    )
)]
pub async fn get(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<axum::Json<DetailsResponse<ChatResponse>>, HttpError> {
    let chat = state.chats.get(&caller, ChatId::new(id)).await?;
    Ok(axum::Json(DetailsResponse {
        data: serializer::chat(&chat),
    }))
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
        (status = NOT_FOUND, description = "The caller has no such chat", body = ErrorResponse)
    )
)]
pub async fn messages(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<axum::Json<ListResponse<MessageResponse>>, HttpError> {
    let messages = state.chats.messages(&caller, ChatId::new(id)).await?;
    Ok(axum::Json(ListResponse {
        total: messages.len() as u64,
        items: messages.iter().map(serializer::message).collect(),
    }))
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
        (status = BAD_REQUEST, description = "The message or the turn is not valid (for example no model, or over the context window)", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such chat, message, or file", body = ErrorResponse),
        (status = UNPROCESSABLE_ENTITY, description = "The chat is already answering (`CHAT_BUSY`), too many chats are (`TOO_MANY_RUNS`), or the model cannot answer (`MODEL_UNAVAILABLE`)", body = ErrorResponse)
    )
)]
pub async fn send(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
    JsonBody(body): JsonBody<SendMessageRequest>,
) -> Result<Response, HttpError> {
    let chat = ChatId::new(id);
    // The caller's chat first, so another user's chat is a 404 and never `CHAT_BUSY`.
    state.chats.get(&caller, chat).await?;
    let reservation = state.runs.reserve(caller.user_id, chat)?;
    let input = TurnInput::Send {
        text: body.text,
        file_ids: body.file_ids.into_iter().map(FileId::new).collect(),
        edit_from: body.edit_from.map(MessageId::new),
    };
    let turn = state.chats.prepare(&caller, chat, input).await?;
    Ok(run(&state, reservation, turn))
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
