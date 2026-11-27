//! `/api/chats`: chats, messages, turns (server-sent `ChatStreamEvent`s), approvals, and read
//! state (`ChatService`, `ChatEngine`, `RunRegistry`). The walking skeleton (P1.9) wrote
//! `create`, `get`, `messages`, and `send`, roadmap step P2.A the other non-streaming handlers,
//! and P3.1 regenerate, retry, decisions, reattaching (`stream`), and stop. Attachments are in
//! `routes::files`. Deleting a chat stops its live run first, so no run writes into a chat that
//! is gone.
//!
//! Every turn route loads the caller's chat first (another user's chat is a `404`, never
//! `CHAT_BUSY`), reserves the chat, prepares the turn (a failure answers `4xx` and frees the
//! chat), and only then switches to server-sent events. The run goes on when the client
//! disconnects; it reattaches with `GET /chats/{id}/stream?after=<seq>`.
//!
//! The caller's `GET /api/events` streams hear of it: `chat.updated` when a chat is created,
//! changed, read, or deleted here, and the run's sink sends `run.started`, `approval.pending`
//! (a run that paused for an approval), and `run.finished` for every run.

pub mod cursor;
#[cfg(test)]
pub(crate) mod flows;
pub mod serializer;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) mod tools;

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
    ProjectResponse, SendMessageRequest, StreamQuery, ToolOverride, UpdateChatRequest, UserEvent,
};
use copilot_core::shared::constants::{
    DEFAULT_PAGE_SIZE, SHUTDOWN_DRAIN_SECS, SSE_KEEP_ALIVE_SECS,
};
use copilot_core::{
    Actor, CallDecision, CallId, ChatId, ChatQuery, ChatUpdate, DomainError, FileId, MessageId,
    ModelId, NewChat, PreparedTurn, ProjectId, ScheduleId, ServerOverride, ToolServerId, TurnInput,
    TurnOutcome, TurnSink, UserId,
};
use copilot_middleware::{HttpError, JsonBody};
use futures_util::StreamExt;
use tokio::task::JoinHandle;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::Caller;
use crate::helpers::runs::{Reservation, Subscription};
use crate::routes::convert::ToCore;
use crate::routes::path::{Id, MessageIds, Params};
use crate::routes::projects;

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

/// Loads the caller's chat, reserves it, prepares the turn `input` asks for, and starts it: the
/// response is the turn's event stream. Another user's chat is a `404`, never `CHAT_BUSY`; a
/// failed `prepare` frees the chat.
///
/// `prepare` and the launch run on a task of their own, which the handler awaits: once `prepare`
/// may have written (the user message, a decision), a client that disconnects can no longer keep
/// the turn from starting (dropping a `JoinHandle` does not cancel its task).
async fn start(
    state: &AppState,
    caller: &Actor,
    chat: ChatId,
    input: TurnInput,
) -> Result<Response, HttpError> {
    state.chats.get(caller, chat).await?;
    let reservation = state.runs.reserve(caller.user_id, chat)?;
    let task = tokio::spawn({
        let state = state.clone();
        let caller = *caller;
        async move {
            let turn = state.chats.prepare(&caller, chat, input).await?;
            Ok::<_, DomainError>(run(&state, reservation, turn))
        }
    });
    let events = task
        .await
        .map_err(|error| DomainError::internal("api.chats.prepare.panicked", error))??;
    Ok(stream_response(state, events))
}

/// Starts the prepared turn in its reserved slot and returns its events. The run goes on when
/// the client disconnects; it stops on shutdown. The slot is released when the run ends.
fn run(state: &AppState, reservation: Reservation, turn: PreparedTurn) -> Subscription {
    let events = reservation.subscribe();
    launch(state, reservation, turn);
    events
}

/// Starts the prepared turn in its reserved slot ([`Reservation::start`]: streams may attach
/// from now on) and runs it on a task of its own, and returns that task: its output is how the
/// run ended, `None` when it failed with an error or panicked (dropping the reservation then
/// ends its streams with `error` and `done`). Nothing cancels the run but
/// [`RunRegistry::stop`] and shutdown; the slot is released when the run ends.
///
/// [`RunRegistry::stop`]: crate::helpers::runs::RunRegistry::stop
pub(crate) fn launch(
    state: &AppState,
    reservation: Reservation,
    turn: PreparedTurn,
) -> JoinHandle<Option<TurnOutcome>> {
    let engine = Arc::clone(&state.engine);
    let chat = turn.chat.id();
    reservation.start();
    tokio::spawn(async move {
        let task = tokio::spawn({
            let sink: Arc<dyn TurnSink> = reservation.sink();
            let cancel = reservation.cancel();
            async move { engine.run(turn, sink, cancel).await }
        });
        let outcome = match task.await {
            Ok(Ok(outcome)) => {
                tracing::info!(chat_id = %chat, status = %outcome.status, "api.chats.run.finished");
                Some(outcome)
            }
            Ok(Err(error)) => {
                tracing::error!(chat_id = %chat, error = %error, "api.chats.run.failed");
                None
            }
            Err(error) => {
                tracing::error!(chat_id = %chat, error = %error, "api.chats.run.panicked");
                None
            }
        };
        // A run that panicked never sent `done`: dropping the reservation ends its streams.
        drop(reservation);
        outcome
    })
}

/// Tells the owner's event streams that `chat` changed.
fn changed(state: &AppState, owner: UserId, chat: ChatId) {
    state.runs.publish(
        owner,
        UserEvent::ChatUpdated {
            chat_id: chat.into(),
        },
    );
}

/// A run's events as server-sent events, each with its `seq` as the event id. The body ends
/// after `done`, or when the server shuts down.
fn stream_response(state: &AppState, events: Subscription) -> Response {
    let events = events
        .into_stream()
        .map(|event| Ok::<_, Infallible>(sse(&event)))
        .take_until(state.shutdown.clone().cancelled_owned());
    Sse::new(events)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(SSE_KEEP_ALIVE_SECS)))
        .into_response()
}

/// A chat's tool server overrides as core reads them.
fn overrides(rows: Vec<ToolOverride>) -> Vec<ServerOverride> {
    rows.into_iter()
        .map(|row| ServerOverride {
            server_id: ToolServerId::new(row.server_id),
            enabled: row.enabled,
        })
        .collect()
}

/// One event as `id: <seq>` and `data: <json>`.
fn sse(event: &ChatStreamEvent) -> Event {
    Event::default()
        .id(event.seq().to_string())
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
        (status = BAD_REQUEST, description = "A query parameter or the cursor is not valid", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such project or schedule", body = ErrorResponse)
    )
)]
pub async fn list(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Params(query): Params<ChatListQuery>,
) -> Result<axum::Json<PageResponse<ChatSummaryResponse>>, HttpError> {
    let cursor = query.cursor.as_deref().map(cursor::decode).transpose()?;
    let page = state
        .chats
        .list(
            &caller,
            ChatQuery {
                project: query.project_id.map(ProjectId::new),
                schedule: query.schedule_id.map(ScheduleId::new),
                pinned: query.pinned,
                scheduled: query.scheduled,
                cursor,
                limit: query.limit.unwrap_or(DEFAULT_PAGE_SIZE),
            },
        )
        .await?;
    Ok(axum::Json(PageResponse {
        items: page.items.iter().map(serializer::summary).collect(),
        next: page.next.map(cursor::encode),
    }))
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
        tool_overrides: overrides(body.tool_overrides.unwrap_or_default()),
        voice_replies: body.voice_replies.unwrap_or(false),
    };
    let chat = state.chats.create(&caller, input).await?;
    changed(&state, caller.user_id, chat.id());
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
        (status = BAD_REQUEST, description = "A field is not valid", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such chat, project, model, or tool server", body = ErrorResponse)
    )
)]
pub async fn update(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
    JsonBody(body): JsonBody<UpdateChatRequest>,
) -> Result<axum::Json<DetailsResponse<ChatResponse>>, HttpError> {
    let changes = ChatUpdate {
        title: body.title,
        project_id: body.project_id.map(|id| id.map(ProjectId::new)),
        model_id: body.model_id.map(|id| id.map(ModelId::new)),
        instructions: body.instructions,
        reasoning: body.reasoning.map(|level| level.map(ToCore::to_core)),
        builtin_tools: body
            .builtin_tools
            .map(|tools| tools.map(|tools| tools.into_iter().map(ToCore::to_core).collect())),
        tool_overrides: body.tool_overrides.map(overrides),
        pinned: body.pinned,
        voice_replies: body.voice_replies,
    };
    let chat = state
        .chats
        .update(&caller, ChatId::new(id), changes)
        .await?;
    changed(&state, caller.user_id, chat.id());
    Ok(axum::Json(DetailsResponse {
        data: serializer::chat(&chat),
    }))
}

#[utoipa::path(
    delete,
    path = "/chats/{id}",
    operation_id = "chats_delete",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id")),
    responses(
        (status = NO_CONTENT, description = "Deleted with its messages and attachments; a running answer stops first"),
        (status = NOT_FOUND, description = "The caller has no such chat", body = ErrorResponse)
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<StatusCode, HttpError> {
    let chat = ChatId::new(id);
    // The caller's chat first: another user's chat is a 404 and its run is never touched.
    state.chats.get(&caller, chat).await?;
    if !state
        .runs
        .stop(chat, Duration::from_secs(SHUTDOWN_DRAIN_SECS))
        .await
    {
        tracing::warn!(chat_id = %chat, "api.chats.delete.run-live");
    }
    state.chats.delete(&caller, chat).await?;
    changed(&state, caller.user_id, chat);
    Ok(StatusCode::NO_CONTENT)
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
        (status = NOT_FOUND, description = "The caller has no such chat", body = ErrorResponse)
    )
)]
pub async fn read(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<StatusCode, HttpError> {
    let chat = ChatId::new(id);
    state.chats.mark_read(&caller, chat).await?;
    // The caller's other devices clear the unread mark.
    changed(&state, caller.user_id, chat);
    Ok(StatusCode::NO_CONTENT)
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
    let input = TurnInput::Send {
        text: body.text,
        file_ids: body.file_ids.into_iter().map(FileId::new).collect(),
        edit_from: body.edit_from.map(MessageId::new),
    };
    start(&state, &caller, ChatId::new(id), input).await
}

#[utoipa::path(
    post,
    path = "/chats/{id}/regenerate",
    operation_id = "chats_regenerate",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id")),
    responses(
        (status = OK, description = "The new answer as server-sent events: every message after the last user message is replaced", content_type = "text/event-stream", body = ChatStreamEvent),
        (status = BAD_REQUEST, description = "The chat has no user message, or the turn is not valid (for example no model)", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such chat", body = ErrorResponse),
        (status = UNPROCESSABLE_ENTITY, description = "The chat is already answering (`CHAT_BUSY`), too many chats are (`TOO_MANY_RUNS`), the model cannot answer (`MODEL_UNAVAILABLE`), or the server is shutting down (`SERVER_STOPPING`)", body = ErrorResponse)
    )
)]
pub async fn regenerate(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<Response, HttpError> {
    start(&state, &caller, ChatId::new(id), TurnInput::Regenerate).await
}

#[utoipa::path(
    post,
    path = "/chats/{id}/retry",
    operation_id = "chats_retry",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id")),
    responses(
        (status = OK, description = "The retried turn as server-sent events: the failed answer and the results of its calls are replaced; earlier rounds of the turn stay", content_type = "text/event-stream", body = ChatStreamEvent),
        (status = BAD_REQUEST, description = "The turn is not valid (for example no model)", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such chat", body = ErrorResponse),
        (status = UNPROCESSABLE_ENTITY, description = "The turn's last answer did not fail (`NOTHING_TO_RETRY`), the chat is already answering (`CHAT_BUSY`), too many chats are (`TOO_MANY_RUNS`), the model cannot answer (`MODEL_UNAVAILABLE`), or the server is shutting down (`SERVER_STOPPING`)", body = ErrorResponse)
    )
)]
pub async fn retry(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<Response, HttpError> {
    start(&state, &caller, ChatId::new(id), TurnInput::Retry).await
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
        (status = OK, description = "The continued turn as server-sent events: the decided calls run (or get their denial) from the paused turn's snapshot, then the model answers", content_type = "text/event-stream", body = ChatStreamEvent),
        (status = BAD_REQUEST, description = "The decisions do not cover exactly the pending calls, a reason is too long, or the turn is not valid (for example no model)", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such chat or message", body = ErrorResponse),
        (status = UNPROCESSABLE_ENTITY, description = "The message does not await a decision (`NOT_AWAITING_APPROVAL`, also when another decision came first), the chat is already answering (`CHAT_BUSY`), too many chats are (`TOO_MANY_RUNS`), the model cannot answer (`MODEL_UNAVAILABLE`), or the server is shutting down (`SERVER_STOPPING`)", body = ErrorResponse)
    )
)]
pub async fn decide(
    State(state): State<AppState>,
    Caller(caller): Caller,
    ids: MessageIds,
    JsonBody(body): JsonBody<DecisionsRequest>,
) -> Result<Response, HttpError> {
    let decisions = body
        .decisions
        .into_iter()
        .map(|decision| CallDecision {
            call_id: CallId::new(decision.call_id),
            decision: decision.decision.to_core(),
            scope: decision.scope.unwrap_or_default().to_core(),
            reason: decision.reason,
        })
        .collect();
    let input = TurnInput::Decide {
        message_id: MessageId::new(ids.message),
        decisions,
    };
    start(&state, &caller, ChatId::new(ids.chat), input).await
}

#[utoipa::path(
    get,
    path = "/chats/{id}/stream",
    operation_id = "chats_stream",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id"), StreamQuery),
    responses(
        (status = OK, description = "The running turn's events after `after` (from the start without it), then live ones, ending with `done`", content_type = "text/event-stream", body = ChatStreamEvent),
        (status = NO_CONTENT, description = "No turn is running, or it is still being prepared (the caller's `GET /api/events` streams hear `run.started` when it starts): read the messages"),
        (status = BAD_REQUEST, description = "`after` is not a number", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such chat", body = ErrorResponse)
    )
)]
pub async fn stream(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
    Params(query): Params<StreamQuery>,
) -> Result<Response, HttpError> {
    let chat = ChatId::new(id);
    // The caller's chat first: another user's chat is a 404 and its run is never attached.
    state.chats.get(&caller, chat).await?;
    Ok(match state.runs.attach(chat, query.after.unwrap_or(0)) {
        Some(events) => stream_response(&state, events),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

#[utoipa::path(
    post,
    path = "/chats/{id}/stop",
    operation_id = "chats_stop",
    tag = "chats",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id")),
    responses(
        (status = NO_CONTENT, description = "The running turn stopped and saved its partial answer as `cancelled` (also when no turn was running)"),
        (status = NOT_FOUND, description = "The caller has no such chat", body = ErrorResponse)
    )
)]
pub async fn stop(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<StatusCode, HttpError> {
    let chat = ChatId::new(id);
    // The caller's chat first: another user's chat is a 404 and its run is never touched.
    state.chats.get(&caller, chat).await?;
    // Answers once the cancelled answer is saved, so the client reads it right after.
    if !state
        .runs
        .stop(chat, Duration::from_secs(SHUTDOWN_DRAIN_SECS))
        .await
    {
        tracing::warn!(chat_id = %chat, "api.chats.stop.run-live");
    }
    Ok(StatusCode::NO_CONTENT)
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
        (status = NOT_FOUND, description = "The caller has no such chat", body = ErrorResponse),
        (status = UNPROCESSABLE_ENTITY, description = "The chat is already in a project (`CHAT_IN_PROJECT`)", body = ErrorResponse)
    )
)]
pub async fn save_as_project(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<axum::Json<DetailsResponse<ProjectResponse>>, HttpError> {
    let chat = ChatId::new(id);
    let project = state.chats.save_as_project(&caller, chat).await?;
    changed(&state, caller.user_id, chat);
    Ok(axum::Json(DetailsResponse {
        data: projects::response(&project),
    }))
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
        (status = BAD_REQUEST, description = "The chat has no instructions to save", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such chat", body = ErrorResponse),
        (status = UNPROCESSABLE_ENTITY, description = "The chat is not in a project (`CHAT_NOT_IN_PROJECT`)", body = ErrorResponse)
    )
)]
pub async fn save_instructions(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<axum::Json<DetailsResponse<ProjectResponse>>, HttpError> {
    let project = state
        .chats
        .save_instructions(&caller, ChatId::new(id))
        .await?;
    Ok(axum::Json(DetailsResponse {
        data: projects::response(&project),
    }))
}
