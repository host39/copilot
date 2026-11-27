//! `ChatEngine::run`: a prepared turn, round by round (`docs/design/copilot.md`, Turn Lifecycle
//! steps 3 to 5, Tool Execution and Approvals, Ending a Turn).
//!
//! Each round streams one answer, saves it as one assistant message, and, when it called tools,
//! resolves every call and saves one `tool` message with the results in call order. A round that
//! needs an approval ends the run; nothing waits in memory. Every exit (stop, shutdown, provider
//! failure, refusal, round limit) saves what streamed and gives each finished call a result, so
//! the next request never meets a call without one.

mod answer;
mod calls;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures_util::future::{Either, select};
use futures_util::stream::{self, StreamExt, TryStreamExt};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::domains::chat::{
    Approval, FinishReason, INTERRUPTED, Message, MessageParts, MessageRepository, MessageStatus,
    Part, PreparedTurn, Role, SKIPPED, ToolCallPart, ToolResultPart, ToolStatus,
};
use crate::domains::conversation::approval::{Policy, TOOL_OFF, Verdict, decide};
use crate::domains::conversation::context::{self, ContextInputs};
use crate::domains::conversation::draft::audio_format;
use crate::domains::conversation::engine::answer::{Answer, Ceiling};
use crate::domains::conversation::engine::calls::{builtin, convert, failed, human};
use crate::domains::conversation::events::TurnEvent;
use crate::domains::file::{FileOwner, FileService, MediaType, StoredFile, duration_ms, wav};
use crate::domains::schedule::{RunOutcome, ScheduleService};
use crate::domains::tool::{ToolServer, ToolService};
use crate::ports::clock::Clock;
use crate::ports::llm_gateway::{LlmEvent, LlmGateway, LlmRequest, PartKind, Refusal};
use crate::ports::tool_host::{TOOL_TIMEOUT, TOOL_UNKNOWN, ToolHost, ToolSession};
use crate::ports::turn_sink::TurnSink;
use crate::shared::constants::{
    AUDIO_OUT_SAMPLE_RATE, MAX_PARALLEL_READS, MAX_TOOL_ROUNDS, MCP_CALL_TIMEOUT_SECS,
};
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{CallId, FileId, MessageId, ToolServerId};

/// The result of a call the run stopped before it finished.
pub const STOPPED: &str = "Stopped before the call finished.";
/// The result of a finished call whose answer failed afterwards.
pub const ANSWER_FAILED: &str = "The answer failed before the call ran.";
/// The result of a call in the last allowed round.
pub const TOOL_LIMIT: &str = "Tool round limit reached.";
/// The result of a call the user denied without a reason.
pub const DENIED: &str = "Denied by the user.";
/// The result of an `auto` call of a paused round whose tool asks by the time it is decided.
pub const NEEDS_APPROVAL: &str = "This tool needs approval now. Ask again to run it.";
/// The result of a call that ran out of time.
pub const TIMED_OUT: &str = "The tool did not answer in time.";
/// The error of an answer that ended without a finish.
pub const ENDED_EARLY: &str = "The provider ended the answer early.";
/// The error of an answer the provider finished with an error.
pub const PROVIDER_FAILED: &str = "The provider ended the answer with an error.";
/// The explanation of a refusal the provider did not explain.
pub const REFUSED: &str = "The model declined to answer.";
/// The failure a scheduled run records when it was stopped.
pub const RUN_STOPPED: &str = "The run was stopped.";
/// The name of the chat file that holds a spoken reply.
pub const SPOKEN_REPLY: &str = "spoken-reply.wav";

/// Runs prepared turns. Cheap to clone; the API spawns one run per reserved slot.
#[derive(Clone)]
pub struct ChatEngine {
    gateway: Arc<dyn LlmGateway>,
    host: Arc<dyn ToolHost>,
    messages: Arc<dyn MessageRepository>,
    tools: ToolService,
    schedules: ScheduleService,
    files: FileService,
    clock: Arc<dyn Clock>,
}

/// How a run ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnOutcome {
    /// The run's last assistant message (`None` when the run wrote none).
    pub message_id: Option<MessageId>,
    /// `complete`, `awaiting_approval`, `cancelled`, `error`, or `refused`.
    pub status: MessageStatus,
    pub finish_reason: Option<FinishReason>,
    /// Human-readable, for `error` and `refused`.
    pub error: Option<String>,
}

impl From<&TurnOutcome> for RunOutcome {
    /// What a scheduled run records.
    fn from(outcome: &TurnOutcome) -> Self {
        match outcome.status {
            MessageStatus::Complete | MessageStatus::Streaming => Self::Ok,
            MessageStatus::AwaitingApproval => Self::AwaitingApproval,
            MessageStatus::Cancelled => Self::Failed(RUN_STOPPED.to_owned()),
            MessageStatus::Error | MessageStatus::Refused => {
                Self::Failed(outcome.error.clone().unwrap_or_else(|| REFUSED.to_owned()))
            }
        }
    }
}

impl ChatEngine {
    pub fn new(
        gateway: Arc<dyn LlmGateway>,
        host: Arc<dyn ToolHost>,
        messages: Arc<dyn MessageRepository>,
        tools: ToolService,
        schedules: ScheduleService,
        files: FileService,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            gateway,
            host,
            messages,
            tools,
            schedules,
            files,
            clock,
        }
    }

    /// Runs `turn` until it completes, pauses for an approval, fails, or `cancel` fires. Events
    /// go to `sink`, ending with `done` on every exit (after `TurnSink::ended` with the outcome). Provider, tool, and file read failures
    /// end the run with an `error` outcome and a saved `error` answer; `Err` means a message
    /// could not be saved (an `error` event is sent too). Internal errors are logged with their
    /// dotted key (`core.engine.failed`) wherever they become the generic text.
    pub async fn run(
        &self,
        turn: PreparedTurn,
        sink: Arc<dyn TurnSink>,
        cancel: CancellationToken,
    ) -> DomainResult<TurnOutcome> {
        let mut run = Run {
            engine: self,
            turn: &turn,
            sink: sink.as_ref(),
            cancel: &cancel,
            history: turn.history.clone(),
            files: turn.references.clone(),
            contents: HashMap::new(),
            servers: turn.servers.clone(),
            session: Session::Closed,
        };
        let result = run.drive().await;
        run.close().await;
        match &result {
            Ok(outcome) => sink.ended(outcome),
            Err(error) => sink.emit(TurnEvent::Error {
                message: human(error),
                code: error.code().to_owned(),
            }),
        }
        sink.emit(TurnEvent::Done);
        result
    }
}

/// The tool session of a run, opened at its first server call.
enum Session {
    Closed,
    Open {
        session: Box<dyn ToolSession>,
        /// Servers whose credentials no longer open, with the reason.
        unreachable: HashMap<ToolServerId, String>,
    },
    Failed(String),
}

enum Next {
    Round,
    End(TurnOutcome),
}

/// One run's state.
struct Run<'a> {
    engine: &'a ChatEngine,
    turn: &'a PreparedTurn,
    sink: &'a dyn TurnSink,
    cancel: &'a CancellationToken,
    /// The chat's messages so far, including what this run saved.
    history: Vec<Message>,
    /// File metadata for requests, including images this run saved.
    files: HashMap<FileId, StoredFile>,
    /// File bytes read so far.
    contents: HashMap<FileId, Bytes>,
    /// The current settings of the snapshot's servers, read again before each round's calls
    /// are decided.
    servers: Vec<ToolServer>,
    session: Session,
}

impl Run<'_> {
    async fn drive(&mut self) -> DomainResult<TurnOutcome> {
        if let Some(outcome) = self.resume().await? {
            return Ok(outcome);
        }
        loop {
            if self.cancel.is_cancelled() {
                return self.stopped_between_rounds().await;
            }
            let round = self.rounds() + 1;
            if let Next::End(outcome) = self.round(round).await? {
                return Ok(outcome);
            }
        }
    }

    /// Stopped before a round started. The outcome is the turn's last answer; a turn with no
    /// answer yet (stopped right after `prepare`) gets an empty `cancelled` one, so every exit
    /// leaves the same trace.
    async fn stopped_between_rounds(&mut self) -> DomainResult<TurnOutcome> {
        let start = self.current();
        let last = self.history[start..]
            .iter()
            .rev()
            .find(|message| message.role() == Role::Assistant);
        if let Some(last) = last {
            return Ok(TurnOutcome {
                message_id: Some(last.id()),
                status: MessageStatus::Cancelled,
                finish_reason: last.finish_reason(),
                error: None,
            });
        }
        let started = self.start().await?;
        let ceiling = Ceiling::new(context::output_limit(
            &self.turn.model,
            &self.turn.capabilities,
        ));
        self.stopped(&started, Answer::new(started.id(), ceiling))
            .await
    }

    /// Resolves the round a decision left open (the paused message, now `complete`): approved
    /// calls run, denied ones get their reason, and `auto` calls run unless the policy now says
    /// otherwise. A round left open by a failure gets `cancelled` results instead.
    async fn resume(&mut self) -> DomainResult<Option<TurnOutcome>> {
        let Some(last) = self.history.last().cloned() else {
            return Ok(None);
        };
        let calls: Vec<ToolCallPart> = last.calls().cloned().collect();
        if last.role() != Role::Assistant || calls.is_empty() {
            return Ok(None);
        }
        if last.status() != MessageStatus::Complete {
            self.skip(&calls, ToolStatus::Cancelled, STOPPED).await?;
            return Ok(None);
        }
        let policy = self.policy();
        let plan = calls
            .into_iter()
            .map(|call| {
                let verdict = match call.approval {
                    Approval::Denied => Verdict::Deny(
                        call.deny_reason
                            .clone()
                            .unwrap_or_else(|| DENIED.to_owned()),
                    ),
                    Approval::Pending => Verdict::Deny(SKIPPED.to_owned()),
                    // The approval answers an ask, never a failure or a tool turned off since.
                    Approval::Approved => match decide(&call, &policy) {
                        Verdict::Fail(text) => Verdict::Fail(text),
                        Verdict::Deny(reason) if reason == TOOL_OFF => Verdict::Deny(reason),
                        Verdict::Run | Verdict::Ask | Verdict::Deny(_) => Verdict::Run,
                    },
                    Approval::Auto => match decide(&call, &policy) {
                        Verdict::Ask => Verdict::Deny(NEEDS_APPROVAL.to_owned()),
                        other => other,
                    },
                };
                (call, verdict)
            })
            .collect();
        self.execute(&last, plan).await
    }

    /// Saves an empty `streaming` assistant message for the next answer.
    async fn start(&mut self) -> DomainResult<Message> {
        let turn = self.turn;
        let user = turn.actor.user_id;
        let started = Message::assistant(
            user,
            turn.chat.id(),
            turn.model.id(),
            turn.provider.id(),
            self.now(),
        );
        let started = Message::restore(MessageParts {
            context_hash: Some(turn.context.hash),
            ..started.to_parts()
        });
        self.engine.messages.append(user, &started).await?;
        self.sink.emit(TurnEvent::MessageStarted {
            message_id: started.id(),
            role: Role::Assistant,
            window_start_id: turn.context.window_start_id,
        });
        Ok(started)
    }

    /// Streams one answer and saves it.
    async fn round(&mut self, round: u32) -> DomainResult<Next> {
        let turn = self.turn;
        let started = self.start().await?;
        let ceiling = Ceiling::new(context::output_limit(&turn.model, &turn.capabilities));
        let mut answer = Answer::new(started.id(), ceiling);
        let request = match self.request().await {
            Ok(request) => request,
            Err(error) => return self.failed(&started, answer, &error).await,
        };
        let opened = or_cancel(
            self.cancel,
            self.engine.gateway.stream(&turn.target, request),
        );
        let mut stream = match opened.await {
            None => return self.stopped(&started, answer).await.map(Next::End),
            Some(Err(error)) => return self.failed(&started, answer, &error).await,
            Some(Ok(stream)) => stream,
        };
        loop {
            match or_cancel(self.cancel, stream.next()).await {
                None => return self.stopped(&started, answer).await.map(Next::End),
                Some(None) => {
                    let error = DomainError::new(ENDED_EARLY, "PROVIDER_STREAM_ENDED");
                    return self.failed(&started, answer, &error).await;
                }
                Some(Some(Err(error))) => return self.failed(&started, answer, &error).await,
                Some(Some(Ok(event))) => {
                    let finished = matches!(event, LlmEvent::Finished { .. });
                    if let Err(error) = answer.apply(event, &turn.context, self.sink) {
                        drop(stream);
                        return self.failed(&started, answer, &error).await;
                    }
                    if finished {
                        break;
                    }
                }
            }
        }
        drop(stream);
        self.answered(&started, answer, round).await
    }

    /// A finished answer: refused, failed, complete, over the round limit, waiting for
    /// approvals, or with calls to run.
    async fn answered(
        &mut self,
        started: &Message,
        mut answer: Answer,
        round: u32,
    ) -> DomainResult<Next> {
        let Some((reason, refusal)) = answer.finish.take() else {
            let error = DomainError::new(ENDED_EARLY, "PROVIDER_STREAM_ENDED");
            return self.failed(started, answer, &error).await;
        };
        match reason {
            FinishReason::Refusal => {
                let explanation = refusal
                    .as_ref()
                    .map(|refusal| refusal.explanation.trim().to_owned())
                    .filter(|text| !text.is_empty())
                    .unwrap_or_else(|| REFUSED.to_owned());
                let saved = self
                    .save(
                        started,
                        &answer,
                        MessageStatus::Refused,
                        Some(reason),
                        Some(explanation.clone()),
                    )
                    .await?;
                self.finished(&saved, refusal);
                return Ok(Next::End(outcome(&saved)));
            }
            FinishReason::Error => {
                let error = DomainError::new(PROVIDER_FAILED, "PROVIDER_ERROR");
                return self.failed(started, answer, &error).await;
            }
            _ => {}
        }
        let calls = answer.calls();
        if calls.is_empty() {
            let saved = self
                .save(
                    started,
                    &answer,
                    MessageStatus::Complete,
                    Some(reason),
                    None,
                )
                .await?;
            self.finished(&saved, None);
            return Ok(Next::End(outcome(&saved)));
        }
        if round >= MAX_TOOL_ROUNDS {
            let saved = self
                .save(
                    started,
                    &answer,
                    MessageStatus::Complete,
                    Some(FinishReason::ToolLimit),
                    None,
                )
                .await?;
            self.requested(&saved, &[]);
            self.finished(&saved, None);
            self.skip(&calls, ToolStatus::Cancelled, TOOL_LIMIT).await?;
            return Ok(Next::End(outcome(&saved)));
        }

        self.reload().await?;
        let policy = self.policy();
        let verdicts: Vec<Verdict> = calls.iter().map(|call| decide(call, &policy)).collect();
        for part in &mut answer.parts {
            let Part::ToolCall(call) = part else {
                continue;
            };
            let verdict = calls
                .iter()
                .zip(&verdicts)
                .find(|(planned, _)| planned.id == call.id)
                .map(|(_, verdict)| verdict);
            match verdict {
                Some(Verdict::Ask) => call.approval = Approval::Pending,
                Some(Verdict::Deny(reason)) => {
                    call.approval = Approval::Denied;
                    call.deny_reason = Some(reason.clone());
                }
                _ => {}
            }
        }
        let asking: Vec<_> = calls
            .iter()
            .zip(&verdicts)
            .filter(|(_, verdict)| **verdict == Verdict::Ask)
            .map(|(call, _)| call.id)
            .collect();
        if !asking.is_empty() {
            let saved = self
                .save(
                    started,
                    &answer,
                    MessageStatus::AwaitingApproval,
                    Some(reason),
                    None,
                )
                .await?;
            self.requested(&saved, &asking);
            self.finished(&saved, None);
            return Ok(Next::End(outcome(&saved)));
        }
        let saved = self
            .save(
                started,
                &answer,
                MessageStatus::Complete,
                Some(reason),
                None,
            )
            .await?;
        self.requested(&saved, &[]);
        self.finished(&saved, None);
        let plan = saved.calls().cloned().zip(verdicts).collect();
        Ok(match self.execute(&saved, plan).await? {
            Some(outcome) => Next::End(outcome),
            None => Next::Round,
        })
    }

    /// Runs a round's calls in order and saves one `tool` message with every result. Returns
    /// the outcome when `cancel` fired: the call in progress and every later call get
    /// `cancelled` results.
    async fn execute(
        &mut self,
        assistant: &Message,
        plan: Vec<(ToolCallPart, Verdict)>,
    ) -> DomainResult<Option<TurnOutcome>> {
        let turn = self.turn;
        let user = turn.actor.user_id;
        let draft = Message::tool(user, turn.chat.id(), Vec::new(), self.now());
        self.sink.emit(TurnEvent::MessageStarted {
            message_id: draft.id(),
            role: Role::Tool,
            window_start_id: turn.context.window_start_id,
        });
        let mut results = Vec::new();
        let mut stopped = false;
        for (call, verdict) in plan {
            let result = if stopped || self.cancel.is_cancelled() {
                stopped = true;
                ToolResultPart::skipped(call.id, ToolStatus::Cancelled, STOPPED)
            } else {
                match verdict {
                    Verdict::Run => match self.call(&call).await {
                        Some(result) => result,
                        None => {
                            stopped = true;
                            ToolResultPart::skipped(call.id, ToolStatus::Cancelled, STOPPED)
                        }
                    },
                    Verdict::Deny(reason) => {
                        ToolResultPart::skipped(call.id, ToolStatus::Denied, &reason)
                    }
                    Verdict::Fail(text) => failed(&call, &text),
                    Verdict::Ask => ToolResultPart::skipped(call.id, ToolStatus::Denied, SKIPPED),
                }
            };
            self.sink.emit(TurnEvent::ToolFinished {
                message_id: draft.id(),
                call_id: result.call_id,
                status: result.status,
                output: result.content.clone(),
                truncated: result.truncated,
            });
            results.push(result);
        }
        let message = Message::restore(MessageParts {
            parts: results.into_iter().map(Part::ToolResult).collect(),
            updated_at: self.now(),
            ..draft.to_parts()
        });
        self.engine.messages.append(user, &message).await?;
        self.sink.emit(TurnEvent::MessageFinished {
            message_id: message.id(),
            status: MessageStatus::Complete,
            finish_reason: None,
            refusal: None,
            usage: None,
        });
        self.history.push(message);
        Ok(stopped.then(|| TurnOutcome {
            message_id: Some(assistant.id()),
            status: MessageStatus::Cancelled,
            finish_reason: assistant.finish_reason(),
            error: None,
        }))
    }

    /// Runs one call; `None` when `cancel` fired first.
    async fn call(&mut self, call: &ToolCallPart) -> Option<ToolResultPart> {
        let turn = self.turn;
        let Some(server) = call.server_id else {
            let origin = turn.chat.origin();
            let schedules = &self.engine.schedules;
            return or_cancel(self.cancel, builtin(schedules, &turn.actor, &origin, call)).await;
        };
        self.open().await;
        let session = match &self.session {
            Session::Open {
                session,
                unreachable,
            } => match unreachable.get(&server) {
                Some(reason) => return Some(failed(call, reason)),
                None => session,
            },
            Session::Failed(reason) => return Some(failed(call, reason)),
            Session::Closed => return Some(failed(call, ANSWER_FAILED)),
        };
        let arguments = call
            .arguments
            .clone()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let timeout = Duration::from_secs(MCP_CALL_TIMEOUT_SECS);
        let output = or_cancel(
            self.cancel,
            session.call(server, &call.tool_name, arguments, timeout),
        )
        .await?;
        Some(match output {
            Ok(output) => {
                let files = &self.engine.files;
                let (result, saved) =
                    convert(files, &turn.actor, turn.chat.id(), call, output).await;
                self.files
                    .extend(saved.into_iter().map(|file| (file.id(), file)));
                result
            }
            Err(error) if error.code() == TOOL_TIMEOUT => ToolResultPart {
                status: ToolStatus::Timeout,
                ..failed(call, TIMED_OUT)
            },
            Err(error) => {
                if error.code() == TOOL_UNKNOWN {
                    // The stored list still offers the tool: list the server again next turn.
                    // The call's result is the same either way, so a failure here only logs.
                    if let Err(stale) = self.engine.tools.stale(&turn.actor, server).await {
                        tracing::warn!(
                            server_id = %server,
                            error = %stale,
                            "core.engine.tools.stale-failed"
                        );
                    }
                }
                failed(call, &human(&error))
            }
        })
    }

    /// Opens the run's tool session over the snapshot's enabled servers whose credentials open.
    async fn open(&mut self) {
        if !matches!(self.session, Session::Closed) {
            return;
        }
        let turn = self.turn;
        let mut endpoints = Vec::new();
        let mut unreachable = HashMap::new();
        for server in self.servers.iter().filter(|server| server.enabled()) {
            match self.engine.tools.endpoint(&turn.actor, server) {
                Ok(endpoint) => endpoints.push(endpoint),
                Err(error) => {
                    unreachable.insert(server.id(), human(&error));
                }
            }
        }
        self.session = match self.engine.host.open(endpoints).await {
            Ok(session) => Session::Open {
                session,
                unreachable,
            },
            Err(error) => Session::Failed(human(&error)),
        };
    }

    async fn close(&mut self) {
        if let Session::Open { session, .. } = std::mem::replace(&mut self.session, Session::Closed)
        {
            session.close().await;
        }
    }

    /// The next request, built from the snapshot, with the file bytes it needs: one read per
    /// file (the metadata is already in `files`), at most 4 at once. A file that is gone by now
    /// reads as deleted.
    async fn request(&mut self) -> DomainResult<LlmRequest> {
        let turn = self.turn;
        let inputs = ContextInputs {
            settings: &turn.settings,
            chat: &turn.chat,
            project: turn.project.as_ref(),
            schedule: turn.schedule.as_ref(),
            model: &turn.model,
            provider: &turn.provider,
            capabilities: &turn.capabilities,
            mode: turn.mode,
            project_files: &turn.files,
            tools: &turn.tools,
            files: &self.files,
            history: &self.history,
        };
        let draft = context::request(&inputs, &turn.context)?;
        let missing: Vec<FileId> = draft
            .files()
            .into_iter()
            .filter(|id| !self.contents.contains_key(id))
            .collect();
        let files = &self.engine.files;
        let actor = &turn.actor;
        let loaded: Vec<Option<(FileId, Bytes)>> = stream::iter(missing)
            .map(|id| async move {
                match files.bytes(actor, id).await {
                    Ok(bytes) => Ok(Some((id, bytes))),
                    Err(DomainError::EntityNotFound { .. }) => Ok(None),
                    Err(error) => Err(error),
                }
            })
            .buffered(MAX_PARALLEL_READS)
            .try_collect()
            .await?;
        self.contents.extend(loaded.into_iter().flatten());
        Ok(draft.resolve(&self.contents))
    }

    /// The answer failed: saved as `error` with the human message, an `error` event, and
    /// `cancelled` results for its finished calls.
    async fn failed(
        &mut self,
        started: &Message,
        answer: Answer,
        error: &DomainError,
    ) -> DomainResult<Next> {
        let text = human(error);
        let saved = self
            .save(
                started,
                &answer,
                MessageStatus::Error,
                Some(FinishReason::Error),
                Some(text.clone()),
            )
            .await?;
        self.sink.emit(TurnEvent::Error {
            message: text,
            code: error.code().to_owned(),
        });
        self.finished(&saved, None);
        self.skip(&answer.calls(), ToolStatus::Cancelled, ANSWER_FAILED)
            .await?;
        Ok(Next::End(outcome(&saved)))
    }

    /// The run was stopped while streaming: the partial answer saved as `cancelled`, and
    /// `cancelled` results for its finished calls.
    async fn stopped(&mut self, started: &Message, answer: Answer) -> DomainResult<TurnOutcome> {
        let saved = self
            .save(started, &answer, MessageStatus::Cancelled, None, None)
            .await?;
        self.finished(&saved, None);
        self.skip(&answer.calls(), ToolStatus::Cancelled, STOPPED)
            .await?;
        Ok(outcome(&saved))
    }

    /// A `tool` message with a `status` result for each finished call (calls whose arguments
    /// never finished need none).
    async fn skip(
        &mut self,
        calls: &[ToolCallPart],
        status: ToolStatus,
        reason: &str,
    ) -> DomainResult<()> {
        let results: Vec<ToolResultPart> = calls
            .iter()
            .filter(|call| call.complete)
            .map(|call| ToolResultPart::skipped(call.id, status, reason))
            .collect();
        if results.is_empty() {
            return Ok(());
        }
        let turn = self.turn;
        let user = turn.actor.user_id;
        let message = Message::tool(user, turn.chat.id(), results, self.now());
        self.engine.messages.append(user, &message).await?;
        self.sink.emit(TurnEvent::MessageStarted {
            message_id: message.id(),
            role: Role::Tool,
            window_start_id: turn.context.window_start_id,
        });
        for part in message.parts() {
            if let Part::ToolResult(result) = part {
                self.sink.emit(TurnEvent::ToolFinished {
                    message_id: message.id(),
                    call_id: result.call_id,
                    status: result.status,
                    output: result.content.clone(),
                    truncated: result.truncated,
                });
            }
        }
        self.sink.emit(TurnEvent::MessageFinished {
            message_id: message.id(),
            status: MessageStatus::Complete,
            finish_reason: None,
            refusal: None,
            usage: None,
        });
        self.history.push(message);
        Ok(())
    }

    /// Saves the streamed answer with its final status, and its spoken audio first.
    async fn save(
        &mut self,
        started: &Message,
        answer: &Answer,
        status: MessageStatus,
        finish_reason: Option<FinishReason>,
        error: Option<String>,
    ) -> DomainResult<Message> {
        let mut parts = answer.parts.clone();
        if let Some(audio) = self.speech(started, answer).await {
            parts.push(audio);
        }
        let saved = Message::restore(MessageParts {
            status,
            finish_reason,
            parts,
            provider_state: answer.provider_state.clone(),
            usage: answer.usage,
            error,
            updated_at: self.now(),
            ..started.to_parts()
        });
        let user = self.turn.actor.user_id;
        if let Err(error) = self.engine.messages.update(user, &saved).await {
            // Best effort: an answer left `streaming` would read as in progress, and refuse
            // Retry, until the next restart.
            let failed = Message::restore(MessageParts {
                status: MessageStatus::Error,
                error: Some(INTERRUPTED.to_owned()),
                ..saved.to_parts()
            });
            if let Err(again) = self.engine.messages.update(user, &failed).await {
                tracing::warn!(error = %again, message_id = %saved.id(), "core.engine.save.failed");
            }
            return Err(error);
        }
        self.history.push(saved.clone());
        Ok(saved)
    }

    /// Saves what the answer spoke, on every exit, as a 24 kHz mono WAV chat file (the upload
    /// rules apply: size, clip length, and storage quota) and returns its `audio` part, the
    /// answer's last, announced with `part.started`. The transcript is already the answer's text,
    /// so the part carries none. `None` when the answer spoke nothing or the file could not be
    /// saved; the answer keeps its transcript either way.
    async fn speech(&mut self, started: &Message, answer: &Answer) -> Option<Part> {
        let pcm = answer.speech();
        if pcm.is_empty() {
            return None;
        }
        let turn = self.turn;
        let bytes = wav(pcm, AUDIO_OUT_SAMPLE_RATE);
        let duration = duration_ms(MediaType::Wav, &bytes).and_then(|ms| u32::try_from(ms).ok());
        let owner = FileOwner::Chat(turn.chat.id());
        let file = match self
            .engine
            .files
            .upload(&turn.actor, owner, SPOKEN_REPLY, bytes)
            .await
        {
            Ok(file) => file,
            Err(error) => {
                tracing::warn!(
                    message_id = %started.id(),
                    code = error.code(),
                    error = %error,
                    "core.engine.speech.save-failed"
                );
                return None;
            }
        };
        self.sink.emit(TurnEvent::PartStarted {
            message_id: started.id(),
            index: u32::try_from(answer.parts.len()).unwrap_or(u32::MAX),
            kind: PartKind::Audio,
        });
        let part = Part::Audio {
            file_id: file.id(),
            format: audio_format(file.media()).to_owned(),
            transcript: None,
            duration_ms: duration,
        };
        self.files.insert(file.id(), file);
        Some(part)
    }

    /// `tool.requested` for each call of `message`; `asking` lists the calls that wait for a
    /// decision.
    fn requested(&self, message: &Message, asking: &[CallId]) {
        for call in message.calls() {
            self.sink.emit(TurnEvent::ToolRequested {
                message_id: message.id(),
                call_id: call.id,
                server_id: call.server_id,
                tool_name: call.tool_name.clone(),
                title: self.title(call),
                arguments: call
                    .arguments
                    .clone()
                    .unwrap_or_else(|| Value::String(call.arguments_raw.clone())),
                needs_approval: asking.contains(&call.id),
            });
        }
    }

    fn finished(&self, message: &Message, refusal: Option<Refusal>) {
        self.sink.emit(TurnEvent::MessageFinished {
            message_id: message.id(),
            status: message.status(),
            finish_reason: message.finish_reason(),
            refusal,
            usage: message.usage().copied(),
        });
    }

    /// A tool's display title from its server's cached list, else its name.
    fn title(&self, call: &ToolCallPart) -> String {
        call.server_id
            .and_then(|id| self.servers.iter().find(|server| server.id() == id))
            .and_then(|server| server.cache())
            .and_then(|cache| cache.tool(&call.tool_name))
            .and_then(|tool| tool.title.clone())
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| call.tool_name.clone())
    }

    /// Reads the snapshot's servers again, so a server turned off or removed, or a tool set to
    /// `off`, since the turn started counts from the next round's calls on.
    async fn reload(&mut self) -> DomainResult<()> {
        let ids: HashSet<ToolServerId> = self
            .turn
            .context
            .tools
            .iter()
            .filter_map(|tool| tool.server_id)
            .collect();
        if ids.is_empty() {
            return Ok(());
        }
        let servers = self.engine.tools.list(&self.turn.actor).await?;
        self.servers = servers
            .into_iter()
            .filter(|server| ids.contains(&server.id()))
            .collect();
        Ok(())
    }

    fn policy(&self) -> Policy<'_> {
        Policy {
            context: &self.turn.context,
            servers: &self.servers,
            approved: self.turn.chat.approved_tools(),
            mode: self.turn.mode,
        }
    }

    /// Assistant messages with tool calls since the last user message, so the round limit
    /// holds across approval pauses.
    fn rounds(&self) -> u32 {
        let rounds = self.history[self.current()..]
            .iter()
            .filter(|message| message.role() == Role::Assistant && message.calls().next().is_some())
            .count();
        u32::try_from(rounds).unwrap_or(u32::MAX)
    }

    /// Where this turn's answers start: just after the last user message.
    fn current(&self) -> usize {
        self.history
            .iter()
            .rposition(|message| message.role() == Role::User)
            .map_or(0, |index| index + 1)
    }

    fn now(&self) -> DateTime<Utc> {
        self.engine.clock.now()
    }
}

fn outcome(message: &Message) -> TurnOutcome {
    TurnOutcome {
        message_id: Some(message.id()),
        status: message.status(),
        finish_reason: message.finish_reason(),
        error: message.error().map(str::to_owned),
    }
}

/// `future`'s output, or `None` when `cancel` fires first.
async fn or_cancel<F: Future>(cancel: &CancellationToken, future: F) -> Option<F::Output> {
    if cancel.is_cancelled() {
        return None;
    }
    let future = pin!(future);
    let cancelled = pin!(cancel.cancelled());
    match select(future, cancelled).await {
        Either::Left((output, _)) => Some(output),
        Either::Right(_) => None,
    }
}
