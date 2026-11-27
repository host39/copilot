//! `ChatEngine::run` against the fakes, end to end from `ChatService::prepare`.

use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::domains::chat::{
    ApprovedTool, CallDecision, Chat, ChatQuery, ChatRepositories, ChatRepository, ChatService,
    ChatSummary, ChatUpdate, Decision, DecisionScope, NewChat, Page, ResultContent, TurnInput,
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
    ApprovalMode, NewToolServer, ServerOverride, ToolMode, ToolServer, ToolServerRepository,
    ToolServerUpdate,
};
use crate::ports::llm_gateway::{
    AudioOut, LlmMessage, LlmPart, PartKind, RemoteModel, ReportedCapabilities,
};
use crate::ports::tool_host::{TOOL_UNKNOWN, ToolContent, ToolListing, ToolOutput};
use crate::shared::actor::Actor;
use crate::shared::base_url::BaseUrl;
use crate::shared::builtin_tool::BuiltinTool;
use crate::shared::constants::{AUDIO_OUT_MAX_BYTES, MCP_RESULT_MAX_BYTES};
use crate::shared::ids::{ChatId, ModelId, ProjectId, ToolServerId, UserId};
use crate::shared::net::PrivateNetwork;
use crate::shared::reasoning::ReasoningLevel;
use crate::testing::{
    CallStep, FakeCipher, FixedClock, MemoryStore, RecordedCall, RecordingSink, ScriptedGateway,
    ScriptedToolHost, Step, actor, call_reply, remote_model, remote_tool, text_reply,
};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n....";

struct Fixture {
    clock: Arc<FixedClock>,
    cipher: Arc<FakeCipher>,
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
    fixture_with(FileLimits::default())
}

fn fixture_with(limits: FileLimits) -> Fixture {
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
    let schedules = ScheduleService::new(
        store.clone(),
        store.clone(),
        store.clone(),
        store.clone(),
        clock.clone(),
    );
    let files = FileService::new(
        store.clone(),
        store.clone(),
        store.clone(),
        store.clone(),
        clock.clone(),
        limits,
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
            cipher.clone(),
            clock.clone(),
            network,
        ),
        projects: ProjectService::new(store.clone(), store.clone(), store.clone(), clock.clone()),
        clock,
        cipher,
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
async fn the_first_answer_names_the_tool_servers_the_turn_lacks() {
    let f = fixture();
    let chat = f.chat(NewChat::default()).await;
    f.gateway.push_events(text_reply("Hello"));
    let mut turn = f.prepare(&chat, send("Hi")).await;
    turn.tools.unavailable = vec![crate::domains::tool::UnavailableServer {
        id: crate::shared::ids::ToolServerId::generate(),
        name: "Docs".to_owned(),
    }];
    let (_, events) = f.run(turn).await;
    assert!(matches!(
        &events[0],
        TurnEvent::MessageStarted { unavailable, .. } if unavailable == &["Docs".to_owned()]
    ));
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
async fn the_sink_hears_the_outcome_just_before_done() {
    let f = fixture();
    let (chat, _) = f.tool_chat(ApprovalMode::Ask).await;
    f.gateway
        .push_events(call_reply(&[("github__delete", r#"{"id":2}"#)]));
    let turn = f.prepare(&chat, send("Clean up")).await;
    let sink = Arc::new(RecordingSink::new());
    let outcome = f
        .engine
        .run(turn, sink.clone(), CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(outcome.status, MessageStatus::AwaitingApproval);
    let paused = f.history(&chat).await[1].id();
    assert_eq!(outcome.message_id, Some(paused));
    let events = sink.events();
    assert_eq!(
        sink.outcomes(),
        [(outcome, events.len() - 1)],
        "once, with only `done` after it"
    );
    assert!(matches!(events.last(), Some(TurnEvent::Done)));
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
async fn an_answer_past_its_output_limit_ends_as_a_bad_response() {
    let f = fixture();
    let chat = f.chat(NewChat::default()).await;
    // The model reports no limits, so the answer may stream 8192 tokens of 16 bytes.
    let chunk = "x".repeat(65_536);
    f.gateway.push_events(vec![
        LlmEvent::TextDelta {
            index: 0,
            text: chunk.clone(),
        },
        LlmEvent::TextDelta {
            index: 0,
            text: chunk.clone(),
        },
        LlmEvent::TextDelta {
            index: 0,
            text: "y".to_owned(),
        },
        LlmEvent::Finished {
            reason: FinishReason::Stop,
            refusal: None,
        },
    ]);
    let (outcome, events) = f.send(&chat, "Hi").await;
    assert_eq!(outcome.status, MessageStatus::Error);
    assert_eq!(outcome.error.as_deref(), Some(answer::TOO_LONG));
    assert!(events.contains(&TurnEvent::Error {
        message: answer::TOO_LONG.to_owned(),
        code: "PROVIDER_BAD_RESPONSE".to_owned(),
    }));
    let deltas = events
        .iter()
        .filter(|event| matches!(event, TurnEvent::TextDelta { .. }))
        .count();
    assert_eq!(deltas, 2, "the refused delta is never sent");
    let saved = f.history(&chat).await[1].clone();
    assert_eq!(saved.status(), MessageStatus::Error);
    assert_eq!(
        saved.parts(),
        [text_part(&chunk.repeat(2))],
        "what streamed is kept"
    );
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
async fn a_tool_the_server_no_longer_lists_expires_the_stored_list() {
    let f = fixture();
    let (chat, server) = f.tool_chat(ApprovalMode::Auto).await;
    assert!(!server.needs_refresh(f.clock.now()));
    f.gateway
        .push_events(call_reply(&[("github__search", "{}")]));
    f.host.push_call_error(DomainError::business_rule(
        "The tool server no longer offers this tool.",
        TOOL_UNKNOWN,
    ));
    f.gateway.push_events(text_reply("Sorry"));
    let (outcome, _) = f.send(&chat, "Go").await;
    assert_eq!(outcome.status, MessageStatus::Complete);

    let history = f.history(&chat).await;
    assert_eq!(results(&history[2])[0].status, ToolStatus::Error);
    let stored = f.tools.get(&f.caller, server.id()).await.unwrap().server;
    assert!(
        stored.needs_refresh(f.clock.now()),
        "the next turn lists the server again"
    );
    assert!(stored.listed(), "the old list stays as the fallback");
    assert_eq!(
        stored.cache().map(|cache| cache.fetched_at),
        server.cache().map(|cache| cache.fetched_at),
        "the list keeps the time it was fetched"
    );

    f.host
        .push_tools(vec![remote_tool("search"), remote_tool("delete")]);
    f.gateway.push_events(text_reply("Listed again"));
    f.send(&chat, "Again").await;
    let listed = f.tools.get(&f.caller, server.id()).await.unwrap().server;
    assert!(!listed.needs_refresh(f.clock.now()), "a fresh list");
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
    let turn = f
        .chats
        .prepare_scheduled(firing)
        .await
        .unwrap()
        .ready()
        .unwrap();
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
    let turn = f
        .chats
        .prepare_scheduled(firing)
        .await
        .unwrap()
        .ready()
        .unwrap();
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

/// Collects formatted log lines.
struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn internal_errors_are_logged_with_their_key() {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer({
            let logs = logs.clone();
            move || Capture(logs.clone())
        })
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let f = fixture();
    let chat = f.chat(NewChat::default()).await;
    f.gateway.push_stream_error(DomainError::internal(
        "provider.http.failed",
        "connection reset",
    ));
    let (outcome, _) = f.send(&chat, "Hi").await;
    assert_eq!(outcome.error.as_deref(), Some(FAILED));

    let text = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(text.contains("core.engine.failed"), "{text}");
    assert!(text.contains("provider.http.failed"), "{text}");
    assert!(text.contains("connection reset"), "{text}");
}

/// Passes file and tool server reads to the store, yielding inside each one so reads started
/// together overlap, and records the most reads in flight at once.
struct Gauge {
    store: Arc<MemoryStore>,
    now: AtomicUsize,
    peak: AtomicUsize,
}

impl Gauge {
    fn new(store: Arc<MemoryStore>) -> Self {
        Self {
            store,
            now: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    async fn read<T>(&self, read: impl Future<Output = T>) -> T {
        let now = self.now.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        tokio::task::yield_now().await;
        let output = read.await;
        self.now.fetch_sub(1, Ordering::SeqCst);
        output
    }

    /// The most reads in flight at once since the last call.
    fn peak(&self) -> usize {
        self.peak.swap(0, Ordering::SeqCst)
    }
}

#[async_trait]
impl FileRepository for Gauge {
    async fn list(&self, user: UserId, owner: FileOwner) -> DomainResult<Vec<StoredFile>> {
        self.read(FileRepository::list(self.store.as_ref(), user, owner))
            .await
    }

    async fn get(&self, user: UserId, id: FileId) -> DomainResult<Option<StoredFile>> {
        self.read(FileRepository::get(self.store.as_ref(), user, id))
            .await
    }

    async fn create(&self, file: &StoredFile, bytes: Bytes, quota: u64) -> DomainResult<bool> {
        FileRepository::create(self.store.as_ref(), file, bytes, quota).await
    }

    async fn content(&self, user: UserId, id: FileId) -> DomainResult<Option<Bytes>> {
        self.read(FileRepository::content(self.store.as_ref(), user, id))
            .await
    }

    async fn update(&self, file: &StoredFile) -> DomainResult<()> {
        FileRepository::update(self.store.as_ref(), file).await
    }

    async fn move_to(&self, user: UserId, ids: &[FileId], project: ProjectId) -> DomainResult<()> {
        FileRepository::move_to(self.store.as_ref(), user, ids, project).await
    }

    async fn delete(&self, user: UserId, id: FileId) -> DomainResult<bool> {
        FileRepository::delete(self.store.as_ref(), user, id).await
    }

    async fn usage(&self, user: UserId) -> DomainResult<u64> {
        FileRepository::usage(self.store.as_ref(), user).await
    }
}

#[async_trait]
impl ToolServerRepository for Gauge {
    async fn list(&self, user: UserId) -> DomainResult<Vec<ToolServer>> {
        self.read(ToolServerRepository::list(self.store.as_ref(), user))
            .await
    }

    async fn get(&self, user: UserId, id: ToolServerId) -> DomainResult<Option<ToolServer>> {
        self.read(ToolServerRepository::get(self.store.as_ref(), user, id))
            .await
    }

    async fn insert(&self, server: &ToolServer) -> DomainResult<()> {
        ToolServerRepository::insert(self.store.as_ref(), server).await
    }

    async fn update(&self, server: &ToolServer) -> DomainResult<()> {
        ToolServerRepository::update(self.store.as_ref(), server).await
    }

    async fn delete(&self, user: UserId, id: ToolServerId) -> DomainResult<bool> {
        ToolServerRepository::delete(self.store.as_ref(), user, id).await
    }

    async fn save_listing(
        &self,
        user: UserId,
        id: ToolServerId,
        url: &BaseUrl,
        listing: &ToolListing,
        review_required: bool,
        at: DateTime<Utc>,
    ) -> DomainResult<bool> {
        ToolServerRepository::save_listing(
            self.store.as_ref(),
            user,
            id,
            url,
            listing,
            review_required,
            at,
        )
        .await
    }

    async fn stale(&self, user: UserId, id: ToolServerId) -> DomainResult<()> {
        ToolServerRepository::stale(self.store.as_ref(), user, id).await
    }
}

#[tokio::test]
async fn prepare_and_run_read_at_most_four_rows_at_once() {
    let f = fixture();
    let gauge = Arc::new(Gauge::new(f.store.clone()));
    let files = FileService::new(
        gauge.clone(),
        f.store.clone(),
        f.store.clone(),
        f.store.clone(),
        f.clock.clone(),
        FileLimits::default(),
    );
    let chats = ChatService::new(
        ChatRepositories {
            files: gauge.clone(),
            servers: gauge.clone(),
            ..ChatRepositories::shared(f.store.clone())
        },
        f.tools.clone(),
        f.cipher.clone(),
        f.clock.clone(),
        PrivateNetwork::Admins,
    );
    let engine = ChatEngine::new(
        f.gateway.clone(),
        f.host.clone(),
        f.store.clone(),
        f.tools.clone(),
        f.schedules.clone(),
        files.clone(),
        f.clock.clone(),
    );
    let project = f
        .projects
        .create(
            &f.caller,
            NewProject {
                name: "Docs".to_owned(),
                ..NewProject::default()
            },
        )
        .await
        .unwrap();
    for index in 0..8 {
        files
            .upload(
                &f.caller,
                FileOwner::Project(project.id()),
                &format!("notes-{index}.md"),
                Bytes::from(format!("Note {index}")),
            )
            .await
            .unwrap();
    }
    let chat = f
        .chat(NewChat {
            project_id: Some(project.id()),
            ..NewChat::default()
        })
        .await;
    let mut attached = Vec::new();
    for index in 0..4 {
        let file = files
            .upload(
                &f.caller,
                FileOwner::Chat(chat.id()),
                &format!("draft-{index}.md"),
                Bytes::from(format!("Draft {index}")),
            )
            .await
            .unwrap();
        attached.push(file.id());
    }
    gauge.peak();

    let turn = chats
        .prepare(
            &f.caller,
            chat.id(),
            TurnInput::Send {
                text: "Summarize".to_owned(),
                file_ids: attached,
                edit_from: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        gauge.peak(),
        4,
        "prepare reads the 4 attachments at once and nothing else"
    );

    f.gateway.push_events(text_reply("Done"));
    let sink = Arc::new(RecordingSink::new());
    let outcome = engine
        .run(turn, sink, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(outcome.status, MessageStatus::Complete);
    assert_eq!(
        gauge.peak(),
        4,
        "the engine reads the bytes of 12 files, 4 at once"
    );
    let sent = format!("{:?}", f.gateway.requests()[0].messages);
    assert!(sent.contains("Note 7") && sent.contains("Draft 3"));
}

#[tokio::test]
async fn a_decision_after_a_model_switch_sends_no_reasoning_the_model_cannot_take() {
    let f = fixture();
    let server = f.server(ApprovalMode::Ask).await;
    let thinking = ReportedCapabilities {
        reasoning: Some(true),
        ..ReportedCapabilities::default()
    };
    let chat = f
        .chat_with(
            thinking,
            NewChat {
                reasoning: Some(ReasoningLevel::High),
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
    f.send(&chat, "Search").await;
    let paused = f.history(&chat).await[1].clone();
    assert_eq!(
        f.gateway.requests()[0].reasoning,
        Some(ReasoningLevel::High)
    );

    let plain = f.model(ReportedCapabilities::default()).await;
    f.chats
        .update(
            &f.caller,
            chat.id(),
            ChatUpdate {
                model_id: Some(Some(plain)),
                ..ChatUpdate::default()
            },
        )
        .await
        .unwrap();
    let turn = f.prepare(&chat, approve(&paused)).await;
    assert!(!turn.capabilities.reasoning);
    f.host.push_text("found");
    f.gateway.push_events(text_reply("Done"));
    f.run(turn).await;
    assert_eq!(
        f.gateway.requests()[1].reasoning,
        None,
        "the snapshot's level only goes to a model with reasoning"
    );
}

#[tokio::test]
async fn a_server_added_again_under_the_same_slug_gets_a_new_snapshot() {
    let f = fixture();
    let (chat, old) = f.tool_chat(ApprovalMode::Ask).await;
    f.gateway.push_events(text_reply("Hello"));
    let turn = f.prepare(&chat, send("Hi")).await;
    let first = turn.context.hash;
    f.run(turn).await;

    f.tools.delete(&f.caller, old.id()).await.unwrap();
    let new = f.server(ApprovalMode::Ask).await;
    assert_eq!(new.slug(), old.slug());
    f.chats
        .update(
            &f.caller,
            chat.id(),
            ChatUpdate {
                tool_overrides: Some(vec![ServerOverride {
                    server_id: new.id(),
                    enabled: true,
                }]),
                ..ChatUpdate::default()
            },
        )
        .await
        .unwrap();
    f.gateway
        .push_events(call_reply(&[("github__search", "{}")]));
    let turn = f.prepare(&chat, send("Search")).await;
    assert_ne!(turn.context.hash, first);
    f.run(turn).await;
    let paused = f.history(&chat).await[3].clone();
    assert_eq!(paused.calls().next().unwrap().server_id, Some(new.id()));

    let turn = f.prepare(&chat, approve(&paused)).await;
    let ids: Vec<_> = turn
        .context
        .tools
        .iter()
        .filter_map(|tool| tool.server_id)
        .collect();
    assert_eq!(ids, [new.id(), new.id()]);
    f.host.push_text("found");
    f.gateway.push_events(text_reply("Done"));
    f.run(turn).await;
    let calls = f.host.calls();
    assert_eq!(calls.len(), 1, "the working server runs the approved call");
    assert_eq!(calls[0].server_id, new.id());
}

/// Approves every pending call of `message` for the rest of the chat.
fn approve_for_chat(message: &Message) -> TurnInput {
    TurnInput::Decide {
        message_id: message.id(),
        decisions: message
            .pending_calls()
            .map(|call| CallDecision {
                call_id: call.id,
                decision: Decision::Approve,
                scope: DecisionScope::Chat,
                reason: None,
            })
            .collect(),
    }
}

/// The store's chats, with a rename saved by another request just before each approval.
struct Renaming {
    store: Arc<MemoryStore>,
}

#[async_trait]
impl ChatRepository for Renaming {
    async fn list(&self, user: UserId, query: ChatQuery) -> DomainResult<Page<ChatSummary>> {
        ChatRepository::list(self.store.as_ref(), user, query).await
    }

    async fn get(&self, user: UserId, id: ChatId) -> DomainResult<Option<Chat>> {
        ChatRepository::get(self.store.as_ref(), user, id).await
    }

    async fn insert(&self, chat: &Chat) -> DomainResult<()> {
        ChatRepository::insert(self.store.as_ref(), chat).await
    }

    async fn update(&self, chat: &Chat) -> DomainResult<()> {
        ChatRepository::update(self.store.as_ref(), chat).await
    }

    async fn set_title(&self, user: UserId, id: ChatId, title: &str) -> DomainResult<bool> {
        ChatRepository::set_title(self.store.as_ref(), user, id, title).await
    }

    async fn clear_instructions(
        &self,
        user: UserId,
        id: ChatId,
        saved: &str,
    ) -> DomainResult<bool> {
        ChatRepository::clear_instructions(self.store.as_ref(), user, id, saved).await
    }

    async fn approve(
        &self,
        user: UserId,
        chat: ChatId,
        tools: &[ApprovedTool],
    ) -> DomainResult<()> {
        let stored = ChatRepository::get(self.store.as_ref(), user, chat)
            .await?
            .expect("the chat exists");
        ChatRepository::update(
            self.store.as_ref(),
            &stored.with_title("Renamed meanwhile")?,
        )
        .await?;
        ChatRepository::approve(self.store.as_ref(), user, chat, tools).await
    }

    async fn delete(&self, user: UserId, id: ChatId) -> DomainResult<bool> {
        ChatRepository::delete(self.store.as_ref(), user, id).await
    }

    async fn mark_read(&self, user: UserId, id: ChatId, at: DateTime<Utc>) -> DomainResult<()> {
        ChatRepository::mark_read(self.store.as_ref(), user, id, at).await
    }
}

#[tokio::test]
async fn approving_for_the_chat_keeps_a_rename_saved_meanwhile() {
    let f = fixture();
    let (chat, server) = f.tool_chat(ApprovalMode::Ask).await;
    f.gateway
        .push_events(call_reply(&[("github__search", "{}")]));
    f.send(&chat, "Search").await;
    let paused = f.history(&chat).await[1].clone();
    let chats = ChatService::new(
        ChatRepositories {
            chats: Arc::new(Renaming {
                store: f.store.clone(),
            }),
            ..ChatRepositories::shared(f.store.clone())
        },
        f.tools.clone(),
        f.cipher.clone(),
        f.clock.clone(),
        PrivateNetwork::Admins,
    );
    let turn = chats
        .prepare(&f.caller, chat.id(), approve_for_chat(&paused))
        .await
        .unwrap();
    assert_eq!(turn.chat.approved_tools().len(), 1, "the turn sees it");

    let stored = f.chats.get(&f.caller, chat.id()).await.unwrap();
    assert_eq!(
        stored.title(),
        "Renamed meanwhile",
        "only approval rows written"
    );
    assert_eq!(
        stored.approved_tools(),
        [ApprovedTool {
            server_id: server.id(),
            name: "search".to_owned(),
        }]
    );
}

#[tokio::test]
async fn a_chat_approval_never_reaches_a_new_server_under_the_same_slug() {
    let f = fixture();
    let (chat, old) = f.tool_chat(ApprovalMode::Ask).await;
    f.gateway
        .push_events(call_reply(&[("github__search", "{}")]));
    f.send(&chat, "Search").await;
    let paused = f.history(&chat).await[1].clone();
    let turn = f.prepare(&chat, approve_for_chat(&paused)).await;
    f.host.push_text("found");
    f.gateway.push_events(text_reply("Done"));
    f.run(turn).await;

    f.gateway
        .push_events(call_reply(&[("github__search", "{}")]));
    f.host.push_text("found again");
    f.gateway.push_events(text_reply("Done"));
    let (outcome, _) = f.send(&chat, "Search again").await;
    assert_eq!(
        outcome.status,
        MessageStatus::Complete,
        "approved for the chat"
    );
    assert_eq!(f.host.calls().len(), 2);

    f.tools.delete(&f.caller, old.id()).await.unwrap();
    let stored = f.chats.get(&f.caller, chat.id()).await.unwrap();
    assert!(
        stored.approved_tools().is_empty(),
        "deleting a server drops its approvals"
    );
    let new = f.server(ApprovalMode::Ask).await;
    f.chats
        .update(
            &f.caller,
            chat.id(),
            ChatUpdate {
                tool_overrides: Some(vec![ServerOverride {
                    server_id: new.id(),
                    enabled: true,
                }]),
                ..ChatUpdate::default()
            },
        )
        .await
        .unwrap();
    f.gateway
        .push_events(call_reply(&[("github__search", "{}")]));
    let (outcome, _) = f.send(&chat, "And now?").await;
    assert_eq!(outcome.status, MessageStatus::AwaitingApproval);
    assert_eq!(f.host.calls().len(), 2, "the new server's tool asks first");
}

#[tokio::test]
async fn a_chat_approval_stays_with_its_tool_when_the_alias_moves() {
    let f = fixture();
    let server = f
        .tools
        .create(
            &f.caller,
            NewToolServer {
                name: "GitHub".to_owned(),
                url: "https://mcp.example.com/mcp".to_owned(),
                ..NewToolServer::default()
            },
        )
        .await
        .unwrap();
    f.host.push_tools(vec![remote_tool("a_b")]);
    f.tools.test(&f.caller, server.id()).await.unwrap();
    let chat = f
        .chat(NewChat {
            tool_overrides: vec![ServerOverride {
                server_id: server.id(),
                enabled: true,
            }],
            ..NewChat::default()
        })
        .await;
    f.gateway.push_events(call_reply(&[("github__a_b", "{}")]));
    f.send(&chat, "Go").await;
    let paused = f.history(&chat).await[1].clone();
    let turn = f.prepare(&chat, approve_for_chat(&paused)).await;
    f.host.push_text("ok");
    f.gateway.push_events(text_reply("Done"));
    f.run(turn).await;
    assert_eq!(f.host.calls()[0].name, "a_b");

    f.host
        .push_tools(vec![remote_tool("a.b"), remote_tool("a_b")]);
    f.tools.test(&f.caller, server.id()).await.unwrap();
    f.tools.review(&f.caller, server.id()).await.unwrap();
    f.gateway.push_events(call_reply(&[("github__a_b", "{}")]));
    let (outcome, _) = f.send(&chat, "Again").await;
    assert_eq!(
        outcome.status,
        MessageStatus::AwaitingApproval,
        "github__a_b now names a.b, which was never approved"
    );
    let asked = f.history(&chat).await[5].calls().next().unwrap().clone();
    assert_eq!(asked.tool_name, "a.b");
    assert_eq!(f.host.calls().len(), 1);
}

#[tokio::test]
async fn retry_keeps_the_turns_earlier_tool_rounds() {
    let f = fixture();
    let (chat, _) = f.tool_chat(ApprovalMode::Auto).await;
    f.gateway
        .push_events(call_reply(&[("github__search", "{}")]));
    f.host.push_text("found");
    let mut steps: Vec<Step> = call_reply(&[("github__delete", r#"{"id":1}"#)])
        .into_iter()
        .filter(|event| !matches!(event, LlmEvent::Finished { .. }))
        .map(|event| Step::Event(Ok(event)))
        .collect();
    steps.push(Step::Event(Err(DomainError::business_rule(
        "Rate limited. Try again in 20 seconds.",
        "PROVIDER_RATE_LIMITED",
    ))));
    f.gateway.push_steps(steps);
    let turn = f.prepare(&chat, send("Clean up")).await;
    let hash = turn.context.hash;
    let (outcome, _) = f.run(turn).await;
    assert_eq!(outcome.status, MessageStatus::Error);
    let history = f.history(&chat).await;
    assert_eq!(
        roles(&history),
        [
            Role::User,
            Role::Assistant,
            Role::Tool,
            Role::Assistant,
            Role::Tool
        ]
    );
    assert_eq!(f.host.calls().len(), 1);

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
    let turn = f.prepare(&chat, TurnInput::Retry).await;
    assert_eq!(turn.context.hash, hash, "the turn keeps its snapshot");
    assert_eq!(
        roles(&turn.history),
        [Role::User, Role::Assistant, Role::Tool]
    );
    f.gateway.push_events(text_reply("Done"));
    let (outcome, _) = f.run(turn).await;
    assert_eq!(outcome.status, MessageStatus::Complete);
    assert_eq!(
        f.host.calls().len(),
        1,
        "the first round's tool never runs again"
    );
    let history = f.history(&chat).await;
    assert_eq!(
        roles(&history),
        [Role::User, Role::Assistant, Role::Tool, Role::Assistant]
    );
    let retried = &f.gateway.requests()[2];
    assert_eq!(
        request_roles(retried),
        [Role::User, Role::Assistant, Role::Tool]
    );
    assert!(!retried.system.contains("Be brief."));

    let error = f
        .chats
        .prepare(&f.caller, chat.id(), TurnInput::Retry)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "NOTHING_TO_RETRY");
}

#[tokio::test]
async fn retry_of_a_first_round_failure_answers_again_with_current_settings() {
    let f = fixture();
    let chat = f.chat(NewChat::default()).await;
    f.gateway.push_stream_error(DomainError::business_rule(
        "Key rejected: open provider settings.",
        "PROVIDER_AUTH",
    ));
    f.send(&chat, "Hi").await;
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
    let turn = f.prepare(&chat, TurnInput::Retry).await;
    f.gateway.push_events(text_reply("Hello"));
    let (outcome, _) = f.run(turn).await;
    assert_eq!(outcome.status, MessageStatus::Complete);
    assert_eq!(
        roles(&f.history(&chat).await),
        [Role::User, Role::Assistant]
    );
    assert!(f.gateway.requests()[1].system.contains("Be brief."));
}

/// A model that speaks (`audio_out`), and a chat with Speak replies on.
async fn speaking_chat(f: &Fixture) -> Chat {
    let reported = ReportedCapabilities {
        audio_out: Some(true),
        ..ReportedCapabilities::default()
    };
    f.chat_with(
        reported,
        NewChat {
            voice_replies: true,
            ..NewChat::default()
        },
    )
    .await
}

fn spoken(data: Vec<u8>, transcript: Option<&str>) -> LlmEvent {
    LlmEvent::AudioDelta {
        data: Bytes::from(data),
        transcript: transcript.map(str::to_owned),
    }
}

fn finished() -> LlmEvent {
    LlmEvent::Finished {
        reason: FinishReason::Stop,
        refusal: None,
    }
}

/// The saved file and bytes of an answer's audio part, with the part's duration.
async fn speech(f: &Fixture, answer: &Message) -> (StoredFile, Bytes, Option<u32>) {
    let (file_id, duration) = answer
        .parts()
        .iter()
        .find_map(|part| match part {
            Part::Audio {
                file_id,
                format,
                transcript,
                duration_ms,
            } => {
                assert_eq!(format, "wav");
                assert_eq!(transcript, &None, "the transcript is the text part");
                Some((*file_id, *duration_ms))
            }
            _ => None,
        })
        .expect("an audio part");
    let file = FileRepository::get(f.store.as_ref(), f.caller.user_id, file_id)
        .await
        .unwrap()
        .expect("the saved file");
    let bytes = FileRepository::content(f.store.as_ref(), f.caller.user_id, file_id)
        .await
        .unwrap()
        .expect("its bytes");
    (file, bytes, duration)
}

#[tokio::test]
async fn spoken_replies_are_saved_as_wav_files_and_replayed_as_transcripts() {
    let f = fixture();
    let chat = speaking_chat(&f).await;
    // Half a second each at 24 kHz PCM16.
    let first: Vec<u8> = (0..24_000_u32).map(|byte| byte as u8).collect();
    let second = vec![7_u8; 24_000];
    f.gateway.push_events(vec![
        spoken(first.clone(), Some("Hel")),
        spoken(second.clone(), Some("lo")),
        spoken(Vec::new(), None),
        finished(),
    ]);
    let (outcome, events) = f.send(&chat, "Say hello").await;
    assert_eq!(outcome.status, MessageStatus::Complete);

    let request = &f.gateway.requests()[0];
    assert_eq!(
        request.audio_out,
        Some(AudioOut {
            voice: "alloy".to_owned(),
            format: "pcm16".to_owned(),
        }),
        "audio is asked for while Speak replies is on"
    );
    let answer = f.history(&chat).await[1].clone();
    assert_eq!(answer.parts().len(), 2);
    assert_eq!(
        answer.parts()[0],
        text_part("Hello"),
        "the transcript is the text"
    );
    let (file, bytes, duration) = speech(&f, &answer).await;
    assert_eq!(duration, Some(1_000));
    assert_eq!(file.name(), SPOKEN_REPLY);
    assert_eq!(file.media_type(), "audio/wav");
    assert_eq!(file.owner(), FileOwner::Chat(chat.id()));
    let pcm = [first, second].concat();
    assert_eq!(bytes, wav(&pcm, 24_000), "a 24 kHz mono PCM16 WAV file");

    assert_eq!(
        kinds(&events),
        [
            "message.started",
            "audio.delta",
            "part.started",
            "text.delta",
            "audio.delta",
            "text.delta",
            "audio.delta",
            "part.started",
            "message.finished",
            "done"
        ]
    );
    assert!(matches!(
        &events[1],
        TurnEvent::AudioDelta { message_id, format, .. }
            if *message_id == answer.id() && format == "pcm16"
    ));
    assert!(matches!(
        &events[2],
        TurnEvent::PartStarted {
            index: 0,
            kind: PartKind::Text,
            ..
        }
    ));
    assert!(matches!(
        &events[7],
        TurnEvent::PartStarted { message_id, index: 1, kind: PartKind::Audio }
            if *message_id == answer.id()
    ));

    // Speak replies off: no audio is asked for, and the reply goes back as its transcript only.
    f.chats
        .update(
            &f.caller,
            chat.id(),
            ChatUpdate {
                voice_replies: Some(false),
                ..ChatUpdate::default()
            },
        )
        .await
        .unwrap();
    f.gateway.push_events(text_reply("Hi"));
    f.send(&chat, "Again").await;
    let next = &f.gateway.requests()[1];
    assert_eq!(next.audio_out, None);
    assert_eq!(
        next.messages[1],
        LlmMessage {
            role: Role::Assistant,
            parts: vec![LlmPart::Text("Hello".to_owned())],
            replay: None,
        }
    );
    assert!(
        next.messages
            .iter()
            .flat_map(|message| &message.parts)
            .all(|part| !matches!(part, LlmPart::Audio { .. })),
        "the spoken audio never goes back"
    );
}

#[tokio::test]
async fn models_without_audio_out_are_never_asked_to_speak() {
    let f = fixture();
    let chat = f
        .chat(NewChat {
            voice_replies: true,
            ..NewChat::default()
        })
        .await;
    f.gateway.push_events(text_reply("Hello"));
    f.send(&chat, "Hi").await;
    assert_eq!(f.gateway.requests()[0].audio_out, None);
}

#[tokio::test]
async fn a_stopped_spoken_reply_keeps_the_audio_it_streamed() {
    let f = fixture();
    let chat = speaking_chat(&f).await;
    f.gateway.push_steps(vec![
        Step::Event(Ok(spoken(vec![1; 4_800], Some("Wait")))),
        Step::Stall,
    ]);
    let turn = f.prepare(&chat, send("Talk")).await;
    let (sink, cancel, handle) = f.spawn(turn);
    until(|| {
        sink.events()
            .iter()
            .any(|event| matches!(event, TurnEvent::AudioDelta { .. }))
    })
    .await;
    cancel.cancel();
    let outcome = handle.await.unwrap().unwrap();
    assert_eq!(outcome.status, MessageStatus::Cancelled);
    let answer = f.history(&chat).await[1].clone();
    assert_eq!(answer.status(), MessageStatus::Cancelled);
    assert_eq!(answer.parts()[0], text_part("Wait"));
    let (_, bytes, duration) = speech(&f, &answer).await;
    assert_eq!(duration, Some(100));
    assert_eq!(bytes.len(), 44 + 4_800);
}

#[tokio::test]
async fn spoken_audio_past_five_minutes_ends_the_answer_and_keeps_what_streamed() {
    let f = fixture();
    let chat = speaking_chat(&f).await;
    let most = usize::try_from(AUDIO_OUT_MAX_BYTES).unwrap();
    f.gateway.push_events(vec![
        spoken(vec![0; most], Some("Long")),
        spoken(vec![0; 2], Some(" more")),
        finished(),
    ]);
    let (outcome, events) = f.send(&chat, "Talk").await;
    assert_eq!(outcome.status, MessageStatus::Error);
    assert_eq!(outcome.error.as_deref(), Some(answer::TOO_LONG));
    let deltas = events
        .iter()
        .filter(|event| matches!(event, TurnEvent::AudioDelta { .. }))
        .count();
    assert_eq!(deltas, 1, "the refused delta is never sent");
    let answer = f.history(&chat).await[1].clone();
    assert_eq!(answer.parts()[0], text_part("Long"));
    let (_, bytes, duration) = speech(&f, &answer).await;
    assert_eq!(duration, Some(300_000), "five minutes fit the clip limit");
    assert_eq!(bytes.len(), 44 + most);
}

#[tokio::test]
async fn a_spoken_reply_that_cannot_be_saved_keeps_its_transcript() {
    let f = fixture_with(FileLimits {
        max_upload_mb: 1,
        max_storage_mb: 1_024,
    });
    let chat = speaking_chat(&f).await;
    // 1.2 MB of audio (25 seconds) does not fit a 1 MB upload limit.
    f.gateway.push_events(vec![
        spoken(vec![0; 1_200_000], Some("Too long to keep")),
        finished(),
    ]);
    let (outcome, events) = f.send(&chat, "Talk").await;
    assert_eq!(outcome.status, MessageStatus::Complete);
    let answer = f.history(&chat).await[1].clone();
    assert_eq!(answer.parts(), [text_part("Too long to keep")]);
    assert!(
        !events.iter().any(|event| matches!(
            event,
            TurnEvent::PartStarted {
                kind: PartKind::Audio,
                ..
            }
        )),
        "no audio part is announced"
    );
    let files = FileRepository::list(
        f.store.as_ref(),
        f.caller.user_id,
        FileOwner::Chat(chat.id()),
    )
    .await
    .unwrap();
    assert!(files.is_empty());
}
