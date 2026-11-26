//! `ChatEngine::run` against the fakes, end to end from `ChatService::prepare`.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::domains::chat::{
    CallDecision, Chat, ChatRepositories, ChatService, ChatUpdate, Decision, DecisionScope,
    NewChat, ResultContent, TurnInput,
};
use crate::domains::conversation::approval::{
    CANNOT_ASK, INVALID_ARGUMENTS, SERVER_GONE, UNKNOWN_TOOL,
};
use crate::domains::conversation::context::{DATA_NOTE, SCHEDULE_NOTE};
use crate::domains::conversation::engine::calls::FAILED;
use crate::domains::file::{FileLimits, FileOwner, FileRepository};
use crate::domains::project::{NewProject, ProjectService};
use crate::domains::provider::{
    ModelQuery, ModelRepository, NewProvider, Preset, ProviderKind, ProviderService,
};
use crate::domains::schedule::{CreatedBy, NewSchedule, OnAsk, ScheduleQuery};
use crate::domains::tool::{
    ApprovalMode, NewToolServer, ServerOverride, ToolMode, ToolServer, ToolServerUpdate,
};
use crate::ports::llm_gateway::{LlmMessage, LlmPart, PartKind, RemoteModel, ReportedCapabilities};
use crate::ports::tool_host::{ToolContent, ToolOutput};
use crate::shared::actor::Actor;
use crate::shared::builtin_tool::BuiltinTool;
use crate::shared::constants::MCP_RESULT_MAX_BYTES;
use crate::shared::ids::ModelId;
use crate::shared::net::PrivateNetwork;
use crate::testing::{
    CallStep, FakeCipher, FixedClock, MemoryStore, RecordedCall, RecordingSink, ScriptedGateway,
    ScriptedToolHost, Step, actor, call_reply, remote_model, remote_tool, text_reply,
};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n....";

struct Fixture {
    chats: ChatService,
    engine: ChatEngine,
    providers: ProviderService,
    projects: ProjectService,
    schedules: ScheduleService,
    tools: ToolService,
    gateway: Arc<ScriptedGateway>,
    host: Arc<ScriptedToolHost>,
    store: Arc<MemoryStore>,
    caller: Actor,
}

fn fixture() -> Fixture {
    let clock = Arc::new(FixedClock::default());
    let store = Arc::new(MemoryStore::new(clock.clone()));
    let gateway = Arc::new(ScriptedGateway::new());
    let host = Arc::new(ScriptedToolHost::new());
    let cipher = Arc::new(FakeCipher::new());
    let network = PrivateNetwork::Admins;
    let tools = ToolService::new(
        store.clone(),
        host.clone(),
        cipher.clone(),
        clock.clone(),
        network,
    );
    let schedules =
        ScheduleService::new(store.clone(), store.clone(), store.clone(), clock.clone());
    let files = FileService::new(
        store.clone(),
        store.clone(),
        store.clone(),
        store.clone(),
        clock.clone(),
        FileLimits::default(),
    );
    let caller = actor();
    store.add_user(&caller);
    Fixture {
        chats: ChatService::new(
            ChatRepositories::shared(store.clone()),
            tools.clone(),
            cipher.clone(),
            clock.clone(),
            network,
        ),
        engine: ChatEngine::new(
            gateway.clone(),
            host.clone(),
            store.clone(),
            tools.clone(),
            schedules.clone(),
            files,
            clock.clone(),
        ),
        providers: ProviderService::new(
            store.clone(),
            store.clone(),
            store.clone(),
            gateway.clone(),
            cipher,
            clock.clone(),
            network,
        ),
        projects: ProjectService::new(store.clone(), store.clone(), store.clone(), clock),
        schedules,
        tools,
        gateway,
        host,
        store,
        caller,
    }
}

impl Fixture {
    /// An enabled OpenAI model `gpt-test` (tool use on unless `reported` says otherwise).
    async fn model(&self, reported: ReportedCapabilities) -> ModelId {
        let provider = self
            .providers
            .create(
                &self.caller,
                NewProvider {
                    kind: ProviderKind::OpenAi,
                    preset: Preset::OpenAi,
                    name: "OpenAI".to_owned(),
                    base_url: None,
                    api_key: Some("sk-1".to_owned()),
                },
            )
            .await
            .unwrap();
        self.gateway.push_models(vec![RemoteModel {
            reported,
            ..remote_model("gpt-test")
        }]);
        self.providers
            .refresh(&self.caller, provider.id())
            .await
            .unwrap();
        let models = ModelRepository::list(
            self.store.as_ref(),
            self.caller.user_id,
            ModelQuery::provider(provider.id()),
        )
        .await
        .unwrap();
        models[0].id()
    }

    async fn chat(&self, input: NewChat) -> Chat {
        self.chat_with(ReportedCapabilities::default(), input).await
    }

    async fn chat_with(&self, reported: ReportedCapabilities, input: NewChat) -> Chat {
        let model = self.model(reported).await;
        self.chats
            .create(
                &self.caller,
                NewChat {
                    model_id: Some(model),
                    ..input
                },
            )
            .await
            .unwrap()
    }

    /// A tested server "GitHub" (slug `github`) with the tools `search` and `delete`.
    async fn server(&self, approval: ApprovalMode) -> ToolServer {
        let server = self
            .tools
            .create(
                &self.caller,
                NewToolServer {
                    name: "GitHub".to_owned(),
                    url: "https://mcp.example.com/mcp".to_owned(),
                    approval,
                    ..NewToolServer::default()
                },
            )
            .await
            .unwrap();
        self.host
            .push_tools(vec![remote_tool("search"), remote_tool("delete")]);
        self.tools
            .test(&self.caller, server.id())
            .await
            .unwrap()
            .server
    }

    /// A standalone chat with the GitHub server turned on.
    async fn tool_chat(&self, approval: ApprovalMode) -> (Chat, ToolServer) {
        let server = self.server(approval).await;
        let chat = self
            .chat(NewChat {
                tool_overrides: vec![ServerOverride {
                    server_id: server.id(),
                    enabled: true,
                }],
                ..NewChat::default()
            })
            .await;
        (chat, server)
    }

    async fn prepare(&self, chat: &Chat, input: TurnInput) -> PreparedTurn {
        self.chats
            .prepare(&self.caller, chat.id(), input)
            .await
            .unwrap()
    }

    async fn run(&self, turn: PreparedTurn) -> (TurnOutcome, Vec<TurnEvent>) {
        let sink = Arc::new(RecordingSink::new());
        let outcome = self
            .engine
            .run(turn, sink.clone(), CancellationToken::new())
            .await
            .unwrap();
        (outcome, sink.events())
    }

    async fn send(&self, chat: &Chat, text: &str) -> (TurnOutcome, Vec<TurnEvent>) {
        let turn = self.prepare(chat, send(text)).await;
        self.run(turn).await
    }

    /// Starts a run in its own task, so the test can cancel it.
    fn spawn(
        &self,
        turn: PreparedTurn,
    ) -> (
        Arc<RecordingSink>,
        CancellationToken,
        tokio::task::JoinHandle<DomainResult<TurnOutcome>>,
    ) {
        let sink = Arc::new(RecordingSink::new());
        let cancel = CancellationToken::new();
        let engine = self.engine.clone();
        let handle = tokio::spawn({
            let sink: Arc<dyn TurnSink> = sink.clone();
            let cancel = cancel.clone();
            async move { engine.run(turn, sink, cancel).await }
        });
        (sink, cancel, handle)
    }

    async fn history(&self, chat: &Chat) -> Vec<Message> {
        MessageRepository::history(self.store.as_ref(), self.caller.user_id, chat.id())
            .await
            .unwrap()
    }
}

fn send(text: &str) -> TurnInput {
    TurnInput::Send {
        text: text.to_owned(),
        file_ids: Vec::new(),
        edit_from: None,
    }
}

/// Approves every pending call of `message` once.
fn approve(message: &Message) -> TurnInput {
    TurnInput::Decide {
        message_id: message.id(),
        decisions: message
            .pending_calls()
            .map(|call| CallDecision {
                call_id: call.id,
                decision: Decision::Approve,
                scope: DecisionScope::Once,
                reason: None,
            })
            .collect(),
    }
}

/// Waits (yielding to the run's task) until `ready` holds.
async fn until(ready: impl Fn() -> bool) {
    for _ in 0..10_000 {
        if ready() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("the run never got there");
}

fn kinds(events: &[TurnEvent]) -> Vec<&'static str> {
    events
        .iter()
        .map(|event| match event {
            TurnEvent::MessageStarted { .. } => "message.started",
            TurnEvent::PartStarted { .. } => "part.started",
            TurnEvent::TextDelta { .. } => "text.delta",
            TurnEvent::ReasoningDelta { .. } => "reasoning.delta",
            TurnEvent::AudioDelta { .. } => "audio.delta",
            TurnEvent::ToolRequested { .. } => "tool.requested",
            TurnEvent::ToolFinished { .. } => "tool.finished",
            TurnEvent::MessageFinished { .. } => "message.finished",
            TurnEvent::Error { .. } => "error",
            TurnEvent::Done => "done",
        })
        .collect()
}

fn roles(messages: &[Message]) -> Vec<Role> {
    messages.iter().map(Message::role).collect()
}

fn request_roles(request: &LlmRequest) -> Vec<Role> {
    request
        .messages
        .iter()
        .map(|message| message.role)
        .collect()
}

fn results(message: &Message) -> Vec<ToolResultPart> {
    message
        .parts()
        .iter()
        .filter_map(|part| match part {
            Part::ToolResult(result) => Some(result.clone()),
            _ => None,
        })
        .collect()
}

fn ok(call: &ToolCallPart, text: &str) -> ToolResultPart {
    ToolResultPart {
        call_id: call.id,
        status: ToolStatus::Ok,
        is_error: false,
        content: vec![ResultContent::Text(text.to_owned())],
        truncated: false,
    }
}

fn text_part(text: &str) -> Part {
    Part::Text {
        text: text.to_owned(),
    }
}

#[tokio::test]
async fn streams_a_text_answer() {
    let f = fixture();
    let chat = f.chat(NewChat::default()).await;
    f.gateway.push_events(text_reply("Hello"));
    let turn = f.prepare(&chat, send("Hi")).await;
    let hash = turn.context.hash;
    let (outcome, events) = f.run(turn).await;

    let history = f.history(&chat).await;
    assert_eq!(roles(&history), [Role::User, Role::Assistant]);
    let answer = &history[1];
    assert_eq!(
        outcome,
        TurnOutcome {
            message_id: Some(answer.id()),
            status: MessageStatus::Complete,
            finish_reason: Some(FinishReason::Stop),
            error: None,
        }
    );
    assert_eq!(answer.status(), MessageStatus::Complete);
    assert_eq!(answer.parts(), [text_part("Hello")]);
    assert_eq!(answer.context_hash(), Some(&hash));
    assert_eq!(answer.model_id(), chat.model_id());
    assert_eq!(
        answer.usage().and_then(|usage| usage.output_tokens),
        Some(5)
    );
    assert_eq!(
        kinds(&events),
        [
            "message.started",
            "part.started",
            "text.delta",
            "message.finished",
            "done"
        ]
    );
    assert!(matches!(
        &events[2],
        TurnEvent::TextDelta { message_id, index: 0, text } if *message_id == answer.id() && text == "Hello"
    ));

    let request = &f.gateway.requests()[0];
    assert_eq!(request.model, "gpt-test");
    assert_eq!(request.system, DATA_NOTE);
    assert_eq!(
        request.messages,
        [LlmMessage {
            role: Role::User,
            parts: vec![LlmPart::Text("Hi".to_owned())],
            replay: None,
        }]
    );
    let target = &f.gateway.stream_targets()[0];
    assert_eq!(target.remote_id, "gpt-test");
    assert_eq!(
        target
            .provider
            .api_key
            .as_ref()
            .map(|key| key.expose().to_vec()),
        Some(b"sk-1".to_vec())
    );
}

#[tokio::test]
async fn keeps_reasoning_and_replays_it_while_the_context_matches() {
    let f = fixture();
    let chat = f.chat(NewChat::default()).await;
    let state = json!({"reasoning_content": "signed"});
    f.gateway.push_events(vec![
        LlmEvent::PartStarted {
            index: 0,
            kind: PartKind::Reasoning,
        },
        LlmEvent::ReasoningDelta {
            index: 0,
            text: "Thinking".to_owned(),
        },
        LlmEvent::TextDelta {
            index: 1,
            text: "Answer".to_owned(),
        },
        LlmEvent::ReplayState(state.clone()),
        LlmEvent::Finished {
            reason: FinishReason::Stop,
            refusal: None,
        },
    ]);
    let (_, events) = f.send(&chat, "Hi").await;
    assert_eq!(
        kinds(&events),
        [
            "message.started",
            "part.started",
            "reasoning.delta",
            "part.started",
            "text.delta",
            "message.finished",
            "done"
        ]
    );
    let history = f.history(&chat).await;
    assert_eq!(
        history[1].parts(),
        [
            Part::Reasoning {
                text: "Thinking".to_owned()
            },
            text_part("Answer")
        ]
    );
    assert_eq!(history[1].provider_state(), Some(&state));

    f.gateway.push_events(text_reply("More"));
    f.send(&chat, "And?").await;
    let second = &f.gateway.requests()[1];
    assert_eq!(
        second.messages[1],
        LlmMessage {
            role: Role::Assistant,
            parts: vec![LlmPart::Text("Answer".to_owned())],
            replay: Some(state.clone()),
        },
        "same provider and context: the provider state goes back, reasoning never as text"
    );

    f.chats
        .update(
            &f.caller,
            chat.id(),
            ChatUpdate {
                instructions: Some("Be brief.".to_owned()),
                ..ChatUpdate::default()
            },
        )
        .await
        .unwrap();
    f.gateway.push_events(text_reply("Ok"));
    f.send(&chat, "Again").await;
    let third = &f.gateway.requests()[2];
    assert_eq!(
        third.messages[1].replay, None,
        "a new context rebuilds earlier answers from their parts"
    );
    assert_eq!(
        third.messages[1].parts,
        [LlmPart::Text("Answer".to_owned())]
    );
}

#[tokio::test]
async fn runs_a_tool_loop() {
    let f = fixture();
    let (chat, server) = f.tool_chat(ApprovalMode::Auto).await;
    f.gateway
        .push_events(call_reply(&[("github__search", r#"{"q":"rust"}"#)]));
    f.host.push_text("found it");
    f.gateway.push_events(text_reply("Done"));
    let (outcome, events) = f.send(&chat, "Search").await;

    let history = f.history(&chat).await;
    assert_eq!(
        roles(&history),
        [Role::User, Role::Assistant, Role::Tool, Role::Assistant]
    );
    assert_eq!(outcome.status, MessageStatus::Complete);
    assert_eq!(outcome.message_id, Some(history[3].id()));
    let call = history[1].calls().next().unwrap().clone();
    assert_eq!(call.tool_name, "search");
    assert_eq!(call.server_id, Some(server.id()));
    assert_eq!(call.provider_call_id.as_deref(), Some("call_1"));
    assert_eq!(call.arguments, Some(json!({"q": "rust"})));
    assert_eq!(call.approval, Approval::Auto);
    assert_eq!(history[1].finish_reason(), Some(FinishReason::ToolCalls));
    assert_eq!(results(&history[2]), [ok(&call, "found it")]);
    assert_eq!(
        f.host.calls(),
        [RecordedCall {
            server_id: server.id(),
            name: "search".to_owned(),
            arguments: json!({"q": "rust"}),
            timeout: Duration::from_secs(60),
        }]
    );
    assert_eq!(f.host.opened().len(), 1, "one session per run");
    assert_eq!(f.host.closed(), 1);

    let requests = f.gateway.requests();
    assert_eq!(requests.len(), 2);
    let aliases: Vec<&str> = requests[0]
        .tools
        .iter()
        .map(|tool| tool.alias.as_str())
        .collect();
    assert_eq!(aliases, ["github__delete", "github__search"]);
    assert_eq!(
        request_roles(&requests[1]),
        [Role::User, Role::Assistant, Role::Tool]
    );
    assert_eq!(
        requests[1].messages[2].parts,
        [LlmPart::ToolResult {
            call_id: call.id.uuid(),
            provider_call_id: format!("call_{}", call.id.uuid().simple()),
            alias: "github__search".to_owned(),
            is_error: false,
            content: vec![ToolContent::Text("found it".to_owned())],
        }]
    );
    assert_eq!(
        kinds(&events),
        [
            "message.started",
            "part.started",
            "tool.requested",
            "message.finished",
            "message.started",
            "tool.finished",
            "message.finished",
            "message.started",
            "part.started",
            "text.delta",
            "message.finished",
            "done"
        ]
    );
    assert!(matches!(
        &events[2],
        TurnEvent::ToolRequested { tool_name, title, needs_approval: false, server_id, .. }
            if tool_name == "search" && title == "search" && *server_id == Some(server.id())
    ));
}

#[tokio::test]
async fn stops_at_the_round_limit_with_tool_limit() {
    let f = fixture();
    let (chat, _) = f.tool_chat(ApprovalMode::Auto).await;
    for _ in 0..MAX_TOOL_ROUNDS {
        f.gateway
            .push_events(call_reply(&[("github__search", "{}")]));
    }
    for _ in 1..MAX_TOOL_ROUNDS {
        f.host.push_text("more");
    }
    let (outcome, _) = f.send(&chat, "Loop").await;

    assert_eq!(outcome.status, MessageStatus::Complete);
    assert_eq!(outcome.finish_reason, Some(FinishReason::ToolLimit));
    let history = f.history(&chat).await;
    let answers: Vec<&Message> = history
        .iter()
        .filter(|message| message.role() == Role::Assistant)
        .collect();
    assert_eq!(answers.len(), MAX_TOOL_ROUNDS as usize);
    let last = answers.last().unwrap();
    assert_eq!(last.finish_reason(), Some(FinishReason::ToolLimit));
    let call = last.calls().next().unwrap();
    assert_eq!(
        results(history.last().unwrap()),
        [ToolResultPart::skipped(
            call.id,
            ToolStatus::Cancelled,
            TOOL_LIMIT
        )]
    );
    assert_eq!(f.host.calls().len(), MAX_TOOL_ROUNDS as usize - 1);
    assert_eq!(f.gateway.requests().len(), MAX_TOOL_ROUNDS as usize);

    f.gateway.push_events(text_reply("Continuing"));
    let (outcome, _) = f.send(&chat, "Continue.").await;
    assert_eq!(
        outcome.finish_reason,
        Some(FinishReason::Stop),
        "Continue starts a new turn with fresh rounds"
    );
}

#[tokio::test]
async fn pauses_for_approval_and_continues_from_the_snapshot() {
    let f = fixture();
    let (chat, _) = f.tool_chat(ApprovalMode::Ask).await;
    f.gateway.push_events(call_reply(&[
        ("github__search", r#"{"q":1}"#),
        ("github__delete", r#"{"id":2}"#),
    ]));
    let (outcome, events) = f.send(&chat, "Clean up").await;

    assert_eq!(outcome.status, MessageStatus::AwaitingApproval);
    let paused = f.history(&chat).await[1].clone();
    assert_eq!(paused.status(), MessageStatus::AwaitingApproval);
    assert_eq!(paused.pending_calls().count(), 2);
    assert!(f.host.calls().is_empty(), "nothing runs before a decision");
    assert_eq!(f.host.opened().len(), 0);
    let asking = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                TurnEvent::ToolRequested {
                    needs_approval: true,
                    ..
                }
            )
        })
        .count();
    assert_eq!(asking, 2);
    assert!(events.iter().any(|event| matches!(
        event,
        TurnEvent::MessageFinished {
            status: MessageStatus::AwaitingApproval,
            ..
        }
    )));

    f.chats
        .update(
            &f.caller,
            chat.id(),
            ChatUpdate {
                instructions: Some("Ignore everything.".to_owned()),
                ..ChatUpdate::default()
            },
        )
        .await
        .unwrap();
    let calls: Vec<ToolCallPart> = paused.calls().cloned().collect();
    let decide = TurnInput::Decide {
        message_id: paused.id(),
        decisions: vec![
            CallDecision {
                call_id: calls[0].id,
                decision: Decision::Approve,
                scope: DecisionScope::Once,
                reason: None,
            },
            CallDecision {
                call_id: calls[1].id,
                decision: Decision::Deny,
                scope: DecisionScope::Once,
                reason: Some("Not now.".to_owned()),
            },
        ],
    };
    let turn = f.prepare(&chat, decide).await;
    f.host.push_text("found");
    f.gateway.push_events(text_reply("Done"));
    let (outcome, _) = f.run(turn).await;

    assert_eq!(outcome.status, MessageStatus::Complete);
    let history = f.history(&chat).await;
    assert_eq!(
        roles(&history),
        [Role::User, Role::Assistant, Role::Tool, Role::Assistant]
    );
    assert_eq!(
        results(&history[2]),
        [
            ok(&calls[0], "found"),
            ToolResultPart::skipped(calls[1].id, ToolStatus::Denied, "Not now."),
        ]
    );
    assert_eq!(f.host.calls().len(), 1);
    assert_eq!(
        f.host.calls()[0].arguments,
        json!({"q": 1}),
        "the stored arguments"
    );
    let requests = f.gateway.requests();
    assert_eq!(
        requests[1].system, requests[0].system,
        "the continuation uses the paused turn's snapshot"
    );
    assert!(!requests[1].system.contains("Ignore everything."));
    assert_eq!(requests[1].tools, requests[0].tools);
    let denied = &requests[1].messages[2].parts[1];
    assert!(matches!(
        denied,
        LlmPart::ToolResult { is_error: true, content, .. }
            if content == &[ToolContent::Text("Not now.".to_owned())]
    ));
}

#[tokio::test]
async fn cancel_saves_the_partial_answer_and_cancelled_results() {
    let f = fixture();
    let (chat, _) = f.tool_chat(ApprovalMode::Auto).await;
    let mut steps: Vec<Step> = call_reply(&[("github__search", "{}")])
        .into_iter()
        .filter(|event| !matches!(event, LlmEvent::Finished { .. }))
        .map(|event| Step::Event(Ok(event)))
        .collect();
    steps.push(Step::Event(Ok(LlmEvent::TextDelta {
        index: 1,
        text: "Partial".to_owned(),
    })));
    steps.push(Step::Stall);
    f.gateway.push_steps(steps);
    let turn = f.prepare(&chat, send("Go")).await;
    let (sink, cancel, handle) = f.spawn(turn);
    until(|| {
        sink.events()
            .iter()
            .any(|event| matches!(event, TurnEvent::TextDelta { .. }))
    })
    .await;
    cancel.cancel();
    let outcome = handle.await.unwrap().unwrap();

    assert_eq!(outcome.status, MessageStatus::Cancelled);
    let history = f.history(&chat).await;
    assert_eq!(roles(&history), [Role::User, Role::Assistant, Role::Tool]);
    let answer = &history[1];
    assert_eq!(answer.status(), MessageStatus::Cancelled);
    let call = answer.calls().next().unwrap().clone();
    assert!(call.complete);
    assert_eq!(answer.parts()[1], text_part("Partial"));
    assert_eq!(
        results(&history[2]),
        [ToolResultPart::skipped(
            call.id,
            ToolStatus::Cancelled,
            STOPPED
        )]
    );
    assert!(f.host.calls().is_empty());
    let events = sink.events();
    assert_eq!(events.last(), Some(&TurnEvent::Done));
    assert!(events.iter().any(|event| matches!(
        event,
        TurnEvent::ToolFinished {
            status: ToolStatus::Cancelled,
            ..
        }
    )));

    f.gateway.push_events(text_reply("Ok"));
    f.send(&chat, "Again").await;
    let next = &f.gateway.requests()[1];
    assert_eq!(
        request_roles(next),
        [Role::User, Role::Assistant, Role::Tool, Role::User],
        "the partial text and the cancelled result go back"
    );
}

#[tokio::test]
async fn cancel_during_a_tool_call_cancels_the_rest_of_the_round() {
    let f = fixture();
    let (chat, _) = f.tool_chat(ApprovalMode::Auto).await;
    f.gateway.push_events(call_reply(&[
        ("github__search", "{}"),
        ("github__delete", "{}"),
    ]));
    f.host.push_step(CallStep::Stall);
    let turn = f.prepare(&chat, send("Go")).await;
    let (_, cancel, handle) = f.spawn(turn);
    until(|| f.host.calls().len() == 1).await;
    cancel.cancel();
    let outcome = handle.await.unwrap().unwrap();

    let history = f.history(&chat).await;
    assert_eq!(roles(&history), [Role::User, Role::Assistant, Role::Tool]);
    assert_eq!(
        outcome,
        TurnOutcome {
            message_id: Some(history[1].id()),
            status: MessageStatus::Cancelled,
            finish_reason: Some(FinishReason::ToolCalls),
            error: None,
        }
    );
    let calls: Vec<&ToolCallPart> = history[1].calls().collect();
    assert_eq!(
        results(&history[2]),
        [
            ToolResultPart::skipped(calls[0].id, ToolStatus::Cancelled, STOPPED),
            ToolResultPart::skipped(calls[1].id, ToolStatus::Cancelled, STOPPED),
        ]
    );
    assert_eq!(f.host.calls().len(), 1, "the second call never ran");
    assert_eq!(f.host.closed(), 1, "the session is closed on every exit");
}

#[tokio::test]
async fn provider_errors_are_saved_as_errors() {
    let f = fixture();
    let chat = f.chat(NewChat::default()).await;
    f.gateway.push_stream_error(DomainError::business_rule(
        "Key rejected: open provider settings.",
        "PROVIDER_AUTH",
    ));
    let (outcome, events) = f.send(&chat, "Hi").await;
    assert_eq!(outcome.status, MessageStatus::Error);
    assert_eq!(
        outcome.error.as_deref(),
        Some("Key rejected: open provider settings.")
    );
    let answer = f.history(&chat).await[1].clone();
    assert_eq!(answer.status(), MessageStatus::Error);
    assert_eq!(answer.finish_reason(), Some(FinishReason::Error));
    assert_eq!(
        answer.error(),
        Some("Key rejected: open provider settings.")
    );
    assert!(events.contains(&TurnEvent::Error {
        message: "Key rejected: open provider settings.".to_owned(),
        code: "PROVIDER_AUTH".to_owned(),
    }));
    assert_eq!(events.last(), Some(&TurnEvent::Done));

    f.gateway.push_events(vec![LlmEvent::TextDelta {
        index: 0,
        text: "Cut".to_owned(),
    }]);
    let (outcome, _) = f.send(&chat, "Again").await;
    assert_eq!(outcome.error.as_deref(), Some(ENDED_EARLY));
    assert_eq!(f.history(&chat).await[3].parts(), [text_part("Cut")]);

    let (tools, _) = f.tool_chat(ApprovalMode::Auto).await;
    let mut steps: Vec<Step> = call_reply(&[("github__search", "{}")])
        .into_iter()
        .filter(|event| !matches!(event, LlmEvent::Finished { .. }))
        .map(|event| Step::Event(Ok(event)))
        .collect();
    steps.push(Step::Event(Err(DomainError::internal(
        "provider.http.failed",
        "connection reset",
    ))));
    f.gateway.push_steps(steps);
    let (outcome, events) = f.send(&tools, "Search").await;
    assert_eq!(
        outcome.error.as_deref(),
        Some(FAILED),
        "internal details stay hidden"
    );
    assert!(events.contains(&TurnEvent::Error {
        message: FAILED.to_owned(),
        code: "INTERNAL_ERROR".to_owned(),
    }));
    let history = f.history(&tools).await;
    assert_eq!(roles(&history), [Role::User, Role::Assistant, Role::Tool]);
    let call = history[1].calls().next().unwrap();
    assert_eq!(
        results(&history[2]),
        [ToolResultPart::skipped(
            call.id,
            ToolStatus::Cancelled,
            ANSWER_FAILED
        )]
    );
    assert!(f.host.calls().is_empty());
}

#[tokio::test]
async fn a_refusal_is_saved_and_left_out_of_later_requests() {
    let f = fixture();
    let chat = f.chat(NewChat::default()).await;
    let refusal = Refusal {
        category: Some("cyber".to_owned()),
        explanation: "I can't help with that.".to_owned(),
    };
    f.gateway.push_events(vec![
        LlmEvent::TextDelta {
            index: 0,
            text: "I".to_owned(),
        },
        LlmEvent::Finished {
            reason: FinishReason::Refusal,
            refusal: Some(refusal.clone()),
        },
    ]);
    let (outcome, events) = f.send(&chat, "Do something bad").await;
    assert_eq!(outcome.status, MessageStatus::Refused);
    assert_eq!(outcome.error.as_deref(), Some("I can't help with that."));
    let answer = f.history(&chat).await[1].clone();
    assert_eq!(answer.status(), MessageStatus::Refused);
    assert_eq!(answer.finish_reason(), Some(FinishReason::Refusal));
    assert_eq!(answer.parts(), [text_part("I")], "kept for display");
    assert!(events.iter().any(|event| matches!(
        event,
        TurnEvent::MessageFinished { status: MessageStatus::Refused, refusal: Some(found), .. }
            if *found == refusal
    )));

    f.gateway.push_events(text_reply("Sure"));
    f.send(&chat, "Something else").await;
    assert_eq!(
        request_roles(&f.gateway.requests()[1]),
        [Role::User, Role::User]
    );
}

#[tokio::test]
async fn tool_failures_become_error_results() {
    let f = fixture();
    let (chat, _) = f.tool_chat(ApprovalMode::Auto).await;
    f.gateway.push_events(call_reply(&[
        ("github__search", "{}"),
        ("github__nope", "{}"),
        ("github__delete", "[1]"),
    ]));
    f.host.push_call_error(DomainError::business_rule(
        "The tool did not answer in time.",
        TOOL_TIMEOUT,
    ));
    f.gateway.push_events(text_reply("Sorry"));
    let (outcome, _) = f.send(&chat, "Go").await;
    assert_eq!(outcome.status, MessageStatus::Complete);

    let history = f.history(&chat).await;
    let statuses: Vec<(ToolStatus, String)> = results(&history[2])
        .into_iter()
        .map(|result| {
            let text = match &result.content[..] {
                [ResultContent::Text(text)] => text.clone(),
                _ => String::new(),
            };
            (result.status, text)
        })
        .collect();
    assert_eq!(
        statuses,
        [
            (ToolStatus::Timeout, TIMED_OUT.to_owned()),
            (ToolStatus::Error, UNKNOWN_TOOL.to_owned()),
            (ToolStatus::Error, INVALID_ARGUMENTS.to_owned()),
        ]
    );
    assert_eq!(
        f.host.calls().len(),
        1,
        "only the known, well-formed call runs"
    );
    let second = &f.gateway.requests()[1];
    assert_eq!(
        request_roles(second),
        [Role::User, Role::Assistant, Role::Tool],
        "an unknown alias still gets its result"
    );
}

#[tokio::test]
async fn tool_images_are_saved_and_long_text_is_truncated() {
    let f = fixture();
    let server = f.server(ApprovalMode::Auto).await;
    let vision = ReportedCapabilities {
        vision: Some(true),
        ..ReportedCapabilities::default()
    };
    let chat = f
        .chat_with(
            vision,
            NewChat {
                tool_overrides: vec![ServerOverride {
                    server_id: server.id(),
                    enabled: true,
                }],
                ..NewChat::default()
            },
        )
        .await;
    f.gateway
        .push_events(call_reply(&[("github__search", "{}")]));
    let link = "A: https://example.com/a";
    f.host.push_output(ToolOutput {
        content: vec![
            ToolContent::Text("x".repeat(MCP_RESULT_MAX_BYTES + 10)),
            ToolContent::Image {
                media_type: "image/png".to_owned(),
                data: Bytes::from_static(PNG),
            },
            ToolContent::Link {
                uri: "https://example.com/a".to_owned(),
                name: Some("A".to_owned()),
            },
        ],
        is_error: false,
    });
    f.gateway.push_events(text_reply("Seen"));
    f.send(&chat, "Show me").await;

    let history = f.history(&chat).await;
    let result = results(&history[2]).remove(0);
    assert!(result.truncated);
    let [
        ResultContent::Text(text),
        ResultContent::Image { file_id },
        ResultContent::Text(note),
    ] = &result.content[..]
    else {
        panic!("unexpected content {:?}", result.content);
    };
    assert_eq!(text.len(), MCP_RESULT_MAX_BYTES);
    assert_eq!(*note, format!("[truncated {} bytes]", 10 + link.len()));
    let saved = FileRepository::get(f.store.as_ref(), f.caller.user_id, *file_id)
        .await
        .unwrap()
        .expect("the image is a chat file");
    assert_eq!(saved.owner(), FileOwner::Chat(chat.id()));

    let second = &f.gateway.requests()[1];
    let LlmPart::ToolResult { content, .. } = &second.messages[2].parts[0] else {
        panic!("a tool result");
    };
    assert_eq!(
        content[1],
        ToolContent::Image {
            media_type: "image/png".to_owned(),
            data: Bytes::from_static(PNG),
        },
        "a vision model gets the image back"
    );
}

#[tokio::test]
async fn schedule_tools_list_without_asking_and_create_after_approval() {
    let f = fixture();
    let chat = f
        .chat(NewChat {
            builtin_tools: Some(vec![BuiltinTool::Schedules]),
            ..NewChat::default()
        })
        .await;
    f.gateway
        .push_events(call_reply(&[("list_schedules", "{}")]));
    f.gateway.push_events(call_reply(&[(
        "schedule_prompt",
        r#"{"name":"Digest","cron":"0 9 * * *","timezone":"UTC","prompt":"News?"}"#,
    )]));
    let (outcome, _) = f.send(&chat, "Schedule a digest").await;
    assert_eq!(outcome.status, MessageStatus::AwaitingApproval);

    let requests = f.gateway.requests();
    let aliases: Vec<&str> = requests[0]
        .tools
        .iter()
        .map(|tool| tool.alias.as_str())
        .collect();
    assert_eq!(
        aliases,
        ["cancel_schedule", "list_schedules", "schedule_prompt"]
    );
    assert!(requests[0].system.contains(SCHEDULE_NOTE));
    let history = f.history(&chat).await;
    let listed = history[1].calls().next().unwrap();
    assert_eq!(
        results(&history[2]),
        [ok(listed, r#"{"schedules":[]}"#)],
        "list_schedules runs without asking"
    );
    let paused = history[3].clone();
    assert_eq!(paused.pending_calls().count(), 1);
    assert!(
        f.schedules
            .list(&f.caller, ScheduleQuery::default())
            .await
            .unwrap()
            .is_empty()
    );

    let turn = f.prepare(&chat, approve(&paused)).await;
    f.gateway.push_events(text_reply("Scheduled."));
    let (outcome, _) = f.run(turn).await;
    assert_eq!(outcome.status, MessageStatus::Complete);
    let schedules = f
        .schedules
        .list(&f.caller, ScheduleQuery::default())
        .await
        .unwrap();
    assert_eq!(schedules.len(), 1);
    let schedule = &schedules[0];
    assert_eq!(schedule.name(), "Digest");
    assert_eq!(schedule.created_by(), CreatedBy::Assistant);
    assert_eq!(schedule.source_chat_id(), Some(chat.id()));
    assert_eq!(schedule.model_id(), chat.model_id());
    let history = f.history(&chat).await;
    let created = results(&history[4]).remove(0);
    assert_eq!(created.status, ToolStatus::Ok);
    let [ResultContent::Text(text)] = &created.content[..] else {
        panic!("a JSON text result");
    };
    let value: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(value["created"]["id"], schedule.id().to_string());
}

#[tokio::test]
async fn unattended_runs_deny_or_pause_ask_tools() {
    let f = fixture();
    let server = f.server(ApprovalMode::Ask).await;
    let model = f.model(ReportedCapabilities::default()).await;
    let project = f
        .projects
        .create(
            &f.caller,
            NewProject {
                name: "Work".to_owned(),
                builtin_tools: vec![BuiltinTool::Schedules],
                tool_server_ids: vec![server.id()],
                ..NewProject::default()
            },
        )
        .await
        .unwrap();
    let schedule = |on_ask: OnAsk| NewSchedule {
        name: "Digest".to_owned(),
        cron: "0 9 * * *".to_owned(),
        timezone: "UTC".to_owned(),
        prompt: "News?".to_owned(),
        project_id: Some(project.id()),
        model_id: Some(model),
        on_ask,
        ..NewSchedule::default()
    };

    let denying = f
        .schedules
        .create(&f.caller, schedule(OnAsk::Deny))
        .await
        .unwrap();
    let firing = f.schedules.run_now(&f.caller, denying.id()).await.unwrap();
    let turn = f.chats.prepare_scheduled(firing).await.unwrap();
    let chat = turn.chat.clone();
    let aliases: Vec<&str> = turn
        .context
        .tools
        .iter()
        .map(|tool| tool.alias.as_str())
        .collect();
    assert_eq!(
        aliases,
        ["github__delete", "github__search"],
        "no schedule tools in unattended runs"
    );
    f.gateway
        .push_events(call_reply(&[("github__search", "{}")]));
    f.gateway.push_events(text_reply("Nothing new."));
    let (outcome, _) = f.run(turn).await;
    assert_eq!(RunOutcome::from(&outcome), RunOutcome::Ok);
    let history = f.history(&chat).await;
    let call = history[1].calls().next().unwrap();
    assert_eq!(call.approval, Approval::Denied);
    assert_eq!(call.deny_reason.as_deref(), Some(CANNOT_ASK));
    assert_eq!(
        results(&history[2]),
        [ToolResultPart::skipped(
            call.id,
            ToolStatus::Denied,
            CANNOT_ASK
        )]
    );
    assert!(f.host.calls().is_empty());

    let pausing = f
        .schedules
        .create(&f.caller, schedule(OnAsk::Pause))
        .await
        .unwrap();
    let firing = f.schedules.run_now(&f.caller, pausing.id()).await.unwrap();
    let turn = f.chats.prepare_scheduled(firing).await.unwrap();
    f.gateway
        .push_events(call_reply(&[("github__search", "{}")]));
    let (outcome, _) = f.run(turn).await;
    assert_eq!(RunOutcome::from(&outcome), RunOutcome::AwaitingApproval);
}

#[test]
fn outcomes_map_to_schedule_run_records() {
    let base = TurnOutcome {
        message_id: None,
        status: MessageStatus::Complete,
        finish_reason: Some(FinishReason::Stop),
        error: None,
    };
    let with = |status: MessageStatus, error: Option<&str>| TurnOutcome {
        status,
        error: error.map(str::to_owned),
        ..base.clone()
    };
    assert_eq!(RunOutcome::from(&base), RunOutcome::Ok);
    assert_eq!(
        RunOutcome::from(&with(MessageStatus::Error, Some("Key rejected."))),
        RunOutcome::Failed("Key rejected.".to_owned())
    );
    assert_eq!(
        RunOutcome::from(&with(MessageStatus::Cancelled, None)),
        RunOutcome::Failed(RUN_STOPPED.to_owned())
    );
    assert_eq!(
        RunOutcome::from(&with(MessageStatus::Refused, None)),
        RunOutcome::Failed(REFUSED.to_owned())
    );
}

#[tokio::test]
async fn an_approval_never_runs_a_tool_turned_off_since() {
    let f = fixture();
    let (chat, server) = f.tool_chat(ApprovalMode::Ask).await;
    f.gateway
        .push_events(call_reply(&[("github__delete", r#"{"id":2}"#)]));
    f.send(&chat, "Delete it").await;
    let paused = f.history(&chat).await[1].clone();
    f.tools
        .update(
            &f.caller,
            server.id(),
            ToolServerUpdate {
                tool_modes: Some([("delete".to_owned(), ToolMode::Off)].into()),
                ..ToolServerUpdate::default()
            },
        )
        .await
        .unwrap();

    let turn = f.prepare(&chat, approve(&paused)).await;
    f.gateway.push_events(text_reply("Ok"));
    let (outcome, _) = f.run(turn).await;
    assert_eq!(outcome.status, MessageStatus::Complete);
    assert!(f.host.calls().is_empty(), "the stale approval card loses");
    let call = paused.calls().next().unwrap();
    assert_eq!(
        results(&f.history(&chat).await[2]),
        [ToolResultPart::skipped(
            call.id,
            ToolStatus::Denied,
            TOOL_OFF
        )]
    );
}

#[tokio::test]
async fn servers_and_tools_turned_off_after_prepare_stop_the_next_calls() {
    let f = fixture();
    let (chat, server) = f.tool_chat(ApprovalMode::Auto).await;
    let turn = f.prepare(&chat, send("Search")).await;
    f.tools
        .update(
            &f.caller,
            server.id(),
            ToolServerUpdate {
                enabled: Some(false),
                ..ToolServerUpdate::default()
            },
        )
        .await
        .unwrap();
    f.gateway
        .push_events(call_reply(&[("github__search", "{}")]));
    f.gateway.push_events(text_reply("Done"));
    f.run(turn).await;
    assert!(f.host.calls().is_empty(), "a disabled server gets no call");
    let history = f.history(&chat).await;
    let call = history[1].calls().next().unwrap();
    assert_eq!(results(&history[2]), [failed(call, SERVER_GONE)]);

    f.tools
        .update(
            &f.caller,
            server.id(),
            ToolServerUpdate {
                enabled: Some(true),
                ..ToolServerUpdate::default()
            },
        )
        .await
        .unwrap();
    let turn = f.prepare(&chat, send("Search again")).await;
    f.tools
        .update(
            &f.caller,
            server.id(),
            ToolServerUpdate {
                tool_modes: Some([("search".to_owned(), ToolMode::Off)].into()),
                ..ToolServerUpdate::default()
            },
        )
        .await
        .unwrap();
    f.gateway
        .push_events(call_reply(&[("github__search", "{}")]));
    f.gateway.push_events(text_reply("Done"));
    f.run(turn).await;
    assert!(f.host.calls().is_empty(), "a tool set to off is denied");
    let history = f.history(&chat).await;
    let call = history[5].calls().next().unwrap();
    assert_eq!(call.approval, Approval::Denied);
    assert_eq!(
        results(&history[6]),
        [ToolResultPart::skipped(
            call.id,
            ToolStatus::Denied,
            TOOL_OFF
        )]
    );
}

#[tokio::test]
async fn a_stop_before_the_first_round_answers_with_an_empty_cancelled_message() {
    let f = fixture();
    let chat = f.chat(NewChat::default()).await;
    f.gateway.push_events(text_reply("Hello"));
    f.send(&chat, "Hi").await;

    let turn = f.prepare(&chat, send("Again")).await;
    let sink = Arc::new(RecordingSink::new());
    let cancel = CancellationToken::new();
    cancel.cancel();
    let outcome = f.engine.run(turn, sink.clone(), cancel).await.unwrap();

    let history = f.history(&chat).await;
    assert_eq!(
        roles(&history),
        [Role::User, Role::Assistant, Role::User, Role::Assistant]
    );
    let stopped = &history[3];
    assert_eq!(stopped.status(), MessageStatus::Cancelled);
    assert!(stopped.parts().is_empty());
    assert_eq!(
        outcome,
        TurnOutcome {
            message_id: Some(stopped.id()),
            status: MessageStatus::Cancelled,
            finish_reason: None,
            error: None,
        },
        "never the previous turn's answer"
    );
    assert_eq!(
        kinds(&sink.events()),
        ["message.started", "message.finished", "done"]
    );
    assert_eq!(f.gateway.requests().len(), 1, "no request was sent");

    f.gateway.push_events(text_reply("Ok"));
    f.send(&chat, "Once more").await;
    assert_eq!(
        request_roles(&f.gateway.requests()[1]),
        [Role::User, Role::Assistant, Role::User, Role::User],
        "the empty answer never goes out"
    );
}
