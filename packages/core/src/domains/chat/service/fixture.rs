//! Services over one `MemoryStore` for the chat service tests.

use std::sync::Arc;

use serde_json::json;

use crate::domains::chat::context::{ContextHash, SnapshotTool, TurnContext};
use crate::domains::chat::entities::Chat;
use crate::domains::chat::message::{
    Approval, FinishReason, Message, MessageParts, MessageStatus, Part, ToolCallPart,
};
use crate::domains::chat::ports::{MessageRepository, TurnContextRepository};
use crate::domains::chat::service::{ChatRepositories, ChatService, NewChat};
use crate::domains::chat::turn::TurnInput;
use crate::domains::file::{FileLimits, FileService};
use crate::domains::project::ProjectService;
use crate::domains::provider::{
    ModelQuery, ModelRepository, ModelService, NewProvider, Preset, ProviderKind, ProviderService,
};
use crate::domains::schedule::ScheduleService;
use crate::domains::tool::{NewToolServer, ToolAlias, ToolService};
use crate::ports::clock::Clock;
use crate::shared::actor::Actor;
use crate::shared::ids::{CallId, ModelId, ProviderId, ToolServerId};
use crate::shared::net::PrivateNetwork;
use crate::testing::{
    FakeCipher, FixedClock, MemoryStore, ScriptedGateway, ScriptedToolHost, remote_model,
    remote_tool,
};

pub(super) struct Fixture {
    pub(super) chats: ChatService,
    pub(super) providers: ProviderService,
    pub(super) models: ModelService,
    pub(super) projects: ProjectService,
    pub(super) files: FileService,
    pub(super) schedules: ScheduleService,
    pub(super) tools: ToolService,
    pub(super) gateway: Arc<ScriptedGateway>,
    pub(super) host: Arc<ScriptedToolHost>,
    pub(super) cipher: Arc<FakeCipher>,
    pub(super) store: Arc<MemoryStore>,
    pub(super) clock: Arc<FixedClock>,
}

pub(super) fn fixture() -> Fixture {
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
    Fixture {
        chats: ChatService::new(
            ChatRepositories::shared(store.clone()),
            tools.clone(),
            cipher.clone(),
            clock.clone(),
            network,
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
        models: ModelService::new(store.clone(), store.clone(), store.clone(), clock.clone()),
        projects: ProjectService::new(store.clone(), store.clone(), store.clone(), clock.clone()),
        files: FileService::new(
            store.clone(),
            store.clone(),
            store.clone(),
            store.clone(),
            clock.clone(),
            FileLimits::default(),
        ),
        schedules: ScheduleService::new(
            store.clone(),
            store.clone(),
            store.clone(),
            store.clone(),
            clock.clone(),
        ),
        tools,
        gateway,
        host,
        cipher,
        store,
        clock,
    }
}

/// An OpenAI connection with key `sk-1` and one enabled model.
pub(super) async fn model(f: &Fixture, caller: &Actor) -> (ModelId, ProviderId) {
    let provider = f
        .providers
        .create(
            caller,
            NewProvider {
                kind: ProviderKind::OpenAi,
                preset: Preset::OpenAi,
                api: None,
                name: "OpenAI".to_owned(),
                base_url: None,
                api_key: Some("sk-1".to_owned()),
            },
        )
        .await
        .unwrap();
    f.gateway.push_models(vec![remote_model("gpt-5.6-terra")]);
    f.providers.refresh(caller, provider.id()).await.unwrap();
    let models = ModelRepository::list(
        f.store.as_ref(),
        caller.user_id,
        ModelQuery::provider(provider.id()),
    )
    .await
    .unwrap();
    (models[0].id(), provider.id())
}

/// A standalone chat with a usable model.
pub(super) async fn chat(f: &Fixture, caller: &Actor) -> Chat {
    let (model, _) = model(f, caller).await;
    let input = NewChat {
        model_id: Some(model),
        ..NewChat::default()
    };
    f.chats.create(caller, input).await.unwrap()
}

/// A tool server listed with one `search` tool.
pub(super) async fn server(f: &Fixture, caller: &Actor) -> ToolServerId {
    let input = NewToolServer {
        name: "GitHub".to_owned(),
        url: "https://mcp.example.com/mcp".to_owned(),
        ..NewToolServer::default()
    };
    let server = f.tools.create(caller, input).await.unwrap();
    f.host.push_tools(vec![remote_tool("search")]);
    f.tools.test(caller, server.id()).await.unwrap();
    server.id()
}

pub(super) fn send(text: &str) -> TurnInput {
    TurnInput::Send {
        text: text.to_owned(),
        file_ids: Vec::new(),
        edit_from: None,
    }
}

fn call(name: &str, approval: Approval) -> ToolCallPart {
    ToolCallPart {
        id: CallId::generate(),
        provider_call_id: Some(format!("call_{name}")),
        server_id: Some(ToolServerId::generate()),
        tool_name: name.to_owned(),
        alias: ToolAlias::restore(format!("gh__{name}")),
        arguments_raw: "{}".to_owned(),
        arguments: Some(json!({})),
        complete: true,
        approval,
        deny_reason: None,
    }
}

/// What the engine leaves after a round that needs approval: an assistant message awaiting it,
/// with two pending calls (`search`, `delete`) and one `auto` call (`read`), and its saved
/// snapshot. Returns the message.
pub(super) async fn pause(f: &Fixture, caller: &Actor, chat: &Chat) -> Message {
    let hash = ContextHash::new([9; 32]);
    let snapshot = TurnContext {
        hash,
        system: "Be brief.".to_owned(),
        tools: ["delete", "read", "search"]
            .into_iter()
            .map(|name| SnapshotTool {
                alias: ToolAlias::restore(format!("gh__{name}")),
                server_id: None,
                name: name.to_owned(),
                description: String::new(),
                input_schema: json!({"type": "object"}),
            })
            .collect(),
        files: Vec::new(),
        window_start_id: None,
        reasoning: None,
    };
    f.store
        .save(caller.user_id, chat.id(), &snapshot)
        .await
        .unwrap();
    let base = Message::assistant(
        caller.user_id,
        chat.id(),
        chat.model_id().unwrap(),
        ProviderId::generate(),
        f.clock.now(),
    );
    let message = Message::restore(MessageParts {
        status: MessageStatus::AwaitingApproval,
        finish_reason: Some(FinishReason::ToolCalls),
        context_hash: Some(hash),
        provider_id: None,
        parts: vec![
            Part::ToolCall(call("search", Approval::Pending)),
            Part::ToolCall(call("delete", Approval::Pending)),
            Part::ToolCall(call("read", Approval::Auto)),
        ],
        ..base.to_parts()
    });
    f.store.append(caller.user_id, &message).await.unwrap();
    message
}
