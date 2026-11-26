use copilot_core::{
    Approval, ApprovedTool, BaseUrl, BuiltinTool, CallId, Chat, ChatRepository, ContextHash,
    FileId, FinishReason, Message, MessageParts, MessageRepository, MessageStatus, NetworkScope,
    Part, ReasoningLevel, Rendering, ResultContent, ServerOverride, Slug, SnapshotFile,
    SnapshotTool, ToolAlias, ToolCallPart, ToolResultPart, ToolServer, ToolServerRepository,
    ToolStatus, TurnContext, TurnContextRepository, Usage, UserId,
};
use copilot_database::{DbPool, PgStore};
use serde_json::json;

use crate::{at, model, store, updated_at, user};

async fn server(store: &PgStore, owner: UserId, slug: &str) -> ToolServer {
    let url = BaseUrl::parse("https://mcp.example.com", NetworkScope::Public).unwrap();
    let server = ToolServer::create(owner, "Docs", Slug::parse(slug).unwrap(), url, at(0)).unwrap();
    ToolServerRepository::insert(store, &server).await.unwrap();
    server
}

async fn chat(store: &PgStore, owner: UserId) -> Chat {
    let chat = Chat::create(owner, at(0));
    ChatRepository::insert(store, &chat).await.unwrap();
    chat
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn chats_round_trip_with_their_overrides_and_approvals(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let model = model(&store, owner).await;
    let docs = server(&store, owner, "docs").await;
    let web = server(&store, owner, "web").await;

    let created = Chat::create(owner, at(0))
        .with_model(Some(model.id()))
        .with_instructions("Be brief.")
        .with_reasoning(Some(ReasoningLevel::High))
        .with_builtin_tools(Some(vec![BuiltinTool::Schedules]))
        .with_tool_overrides(vec![
            ServerOverride {
                server_id: docs.id(),
                enabled: true,
            },
            ServerOverride {
                server_id: web.id(),
                enabled: false,
            },
        ])
        .approving(ApprovedTool {
            server_id: docs.id(),
            name: "search".to_owned(),
        });
    ChatRepository::insert(&store, &created).await.unwrap();
    assert_eq!(
        ChatRepository::get(&store, owner, created.id())
            .await
            .unwrap(),
        Some(created.clone())
    );
    assert_eq!(
        ChatRepository::get(&store, other, created.id())
            .await
            .unwrap(),
        None
    );

    // Settings changes replace the join rows and never move `updated_at` or `last_read_at`.
    let changed = created
        .clone()
        .titled_by("What is Rust?")
        .with_pinned(true)
        .with_builtin_tools(None)
        .with_tool_overrides(vec![ServerOverride {
            server_id: web.id(),
            enabled: true,
        }]);
    let changed = Chat::restore(copilot_core::ChatParts {
        approved_tools: Vec::new(),
        updated_at: at(50),
        last_read_at: Some(at(50)),
        ..changed.to_parts()
    });
    ChatRepository::update(&store, &changed).await.unwrap();
    let stored = ChatRepository::get(&store, owner, created.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.title(), "What is Rust?");
    assert!(stored.pinned());
    assert_eq!(stored.builtin_tools(), None);
    assert_eq!(stored.tool_overrides(), changed.tool_overrides());
    assert!(stored.approved_tools().is_empty());
    assert_eq!(stored.updated_at(), at(0));
    assert_eq!(stored.last_read_at(), Some(at(0)));

    ChatRepository::mark_read(&store, owner, created.id(), at(7))
        .await
        .unwrap();
    ChatRepository::mark_read(&store, other, created.id(), at(9))
        .await
        .unwrap();
    let stored = ChatRepository::get(&store, owner, created.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.last_read_at(),
        Some(at(7)),
        "only the owner marks it read"
    );

    assert!(
        !ChatRepository::delete(&store, other, created.id())
            .await
            .unwrap()
    );
    assert!(
        ChatRepository::delete(&store, owner, created.id())
            .await
            .unwrap()
    );
    assert_eq!(
        ChatRepository::get(&store, owner, created.id())
            .await
            .unwrap(),
        None
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn messages_keep_every_part_and_move_the_chat(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let model = model(&store, owner).await;
    let chat = chat(&store, owner).await;
    let docs = server(&store, owner, "docs").await;

    let question = Message::user(
        owner,
        chat.id(),
        vec![Part::Text {
            text: "Find it".to_owned(),
        }],
        at(10),
    );
    store.append(owner, &question).await.unwrap();
    assert_eq!(updated_at(&store, owner, chat.id()).await, at(10));

    let call = CallId::generate();
    let started = Message::assistant(owner, chat.id(), model.id(), model.provider_id(), at(11));
    store.append(owner, &started).await.unwrap();
    let answered = Message::restore(MessageParts {
        status: MessageStatus::Complete,
        finish_reason: Some(FinishReason::ToolCalls),
        context_hash: Some(ContextHash::new([3; 32])),
        parts: vec![
            Part::Reasoning {
                text: "Searching".to_owned(),
            },
            Part::Text {
                text: "Let me look.".to_owned(),
            },
            Part::ToolCall(ToolCallPart {
                id: call,
                provider_call_id: Some("call_1".to_owned()),
                server_id: Some(docs.id()),
                tool_name: "search".to_owned(),
                alias: ToolAlias::restore("docs__search".to_owned()),
                arguments_raw: "{}".to_owned(),
                arguments: Some(json!({})),
                complete: true,
                approval: Approval::Auto,
                deny_reason: None,
            }),
        ],
        provider_state: Some(json!({"reasoning": "signed"})),
        usage: Some(Usage {
            input_tokens: Some(20),
            output_tokens: Some(5),
            ..Usage::default()
        }),
        updated_at: at(12),
        ..started.to_parts()
    });
    MessageRepository::update(&store, owner, &answered)
        .await
        .unwrap();
    let results = Message::tool(
        owner,
        chat.id(),
        vec![ToolResultPart {
            call_id: call,
            status: ToolStatus::Ok,
            is_error: false,
            content: vec![
                ResultContent::Text("Found.".to_owned()),
                ResultContent::Image {
                    file_id: FileId::generate(),
                },
            ],
            truncated: false,
        }],
        at(13),
    );
    store.append(owner, &results).await.unwrap();

    let history = store.history(owner, chat.id()).await.unwrap();
    assert_eq!(history, vec![question, answered, results], "ordered by seq");
    assert_eq!(updated_at(&store, owner, chat.id()).await, at(13));
    assert!(store.history(other, chat.id()).await.unwrap().is_empty());

    let text: String = sqlx::query_scalar("SELECT text_content FROM messages WHERE id = $1")
        .bind(history[1].id().uuid())
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(text, "Let me look.", "search reads the text parts");
    let state: serde_json::Value =
        sqlx::query_scalar("SELECT provider_state FROM messages WHERE id = $1")
            .bind(history[1].id().uuid())
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(state["v"], 1);
    assert_eq!(state["provider_id"], json!(model.provider_id().uuid()));
    assert_eq!(state["model_id"], json!(model.id().uuid()));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn messages_are_written_only_for_their_owner(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let chat = chat(&store, owner).await;
    let message = Message::user(
        owner,
        chat.id(),
        vec![Part::Text {
            text: "Hi".to_owned(),
        }],
        at(1),
    );
    assert!(
        store.append(other, &message).await.is_err(),
        "a message of another user is refused"
    );
    let foreign = Message::user(
        other,
        chat.id(),
        vec![Part::Text {
            text: "Hi".to_owned(),
        }],
        at(1),
    );
    assert!(
        store.append(other, &foreign).await.is_err(),
        "the chat belongs to someone else (composite foreign key)"
    );
    store.append(owner, &message).await.unwrap();
    let edited = Message::restore(MessageParts {
        parts: vec![Part::Text {
            text: "Changed".to_owned(),
        }],
        ..message.to_parts()
    });
    assert!(
        MessageRepository::update(&store, other, &edited)
            .await
            .is_err()
    );
    assert_eq!(
        store.history(owner, chat.id()).await.unwrap(),
        vec![message]
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn snapshots_are_saved_once_per_hash(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let chat = chat(&store, owner).await;
    let docs = server(&store, owner, "docs").await;
    let context = TurnContext {
        hash: ContextHash::new([5; 32]),
        system: "Be brief.".to_owned(),
        tools: vec![SnapshotTool {
            alias: ToolAlias::restore("docs__search".to_owned()),
            server_id: Some(docs.id()),
            name: "search".to_owned(),
            description: "Search.".to_owned(),
            input_schema: json!({"type": "object"}),
        }],
        files: vec![SnapshotFile {
            file_id: FileId::generate(),
            sha256: [0; 32],
            rendering: Rendering::Deleted,
        }],
        window_start_id: None,
        reasoning: Some(ReasoningLevel::Low),
    };
    TurnContextRepository::save(&store, owner, chat.id(), &context)
        .await
        .unwrap();
    let again = TurnContext {
        system: "Changed.".to_owned(),
        ..context.clone()
    };
    TurnContextRepository::save(&store, owner, chat.id(), &again)
        .await
        .unwrap();
    assert_eq!(
        TurnContextRepository::get(&store, owner, chat.id(), &context.hash)
            .await
            .unwrap(),
        Some(context.clone()),
        "insert-or-ignore by hash"
    );
    assert_eq!(
        TurnContextRepository::get(&store, other, chat.id(), &context.hash)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        TurnContextRepository::get(&store, owner, chat.id(), &ContextHash::new([6; 32]))
            .await
            .unwrap(),
        None
    );
}
