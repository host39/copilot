use chrono::{DateTime, Utc};
use copilot_core::{
    Approval, ApprovedTool, BuiltinTool, CallDecision, CallId, Chat, ChatCursor, ChatId, ChatQuery,
    ChatRepository, ContextHash, Decision, DecisionScope, FileId, FinishReason, Message,
    MessageParts, MessageRepository, MessageStatus, Part, Project, ProjectRepository,
    ReasoningLevel, Rendering, ResultContent, ScheduleId, ServerOverride, SnapshotFile,
    SnapshotTool, ToolAlias, ToolCallPart, ToolResultPart, ToolStatus, TurnContext,
    TurnContextRepository, Usage, UserId,
};
use copilot_database::{DbPool, PgStore};
use serde_json::json;

use crate::schedules::schedule;
use crate::{NOW, at, model, server, store, updated_at, user};

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

    // Settings changes replace the overrides, keep the approvals (they only grow), and never
    // move `updated_at` or `last_read_at`.
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
    assert_eq!(stored.approved_tools(), created.approved_tools());
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
async fn a_stale_chat_update_keeps_later_approvals_and_skips_deleted_servers(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let docs = server(&store, owner, "docs").await;
    let web = server(&store, owner, "web").await;
    let created = Chat::create(owner, at(0)).with_tool_overrides(vec![
        ServerOverride {
            server_id: docs.id(),
            enabled: true,
        },
        ServerOverride {
            server_id: web.id(),
            enabled: true,
        },
    ]);
    ChatRepository::insert(&store, &created).await.unwrap();
    // A rename reads the chat; then "Approve for this chat" saves an approval, and the web
    // server is deleted; then the rename writes what it read.
    let stale = created.clone().titled_by("Renamed");
    let search = ApprovedTool {
        server_id: docs.id(),
        name: "search".to_owned(),
    };
    ChatRepository::update(&store, &created.clone().approving(search.clone()))
        .await
        .unwrap();
    let fetch = ApprovedTool {
        server_id: web.id(),
        name: "fetch".to_owned(),
    };
    ChatRepository::update(&store, &created.clone().approving(fetch))
        .await
        .unwrap();
    assert!(
        copilot_core::ToolServerRepository::delete(&store, owner, web.id())
            .await
            .unwrap()
    );

    ChatRepository::update(&store, &stale).await.unwrap();
    let stored = ChatRepository::get(&store, owner, created.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.title(), "Renamed");
    assert_eq!(
        stored.approved_tools(),
        [search],
        "the approval stays; the deleted server's went with it"
    );
    assert_eq!(
        stored.tool_overrides(),
        [ServerOverride {
            server_id: docs.id(),
            enabled: true,
        }],
        "the deleted server is skipped, not a failed update"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn approve_adds_approval_rows_only(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let docs = server(&store, owner, "docs").await;
    let gone = server(&store, owner, "gone").await;
    let theirs = server(&store, other, "theirs").await;
    let created = chat(&store, owner).await;
    // A rename saved after the decision read the chat stays.
    ChatRepository::update(&store, &created.clone().titled_by("Renamed"))
        .await
        .unwrap();
    assert!(
        copilot_core::ToolServerRepository::delete(&store, owner, gone.id())
            .await
            .unwrap()
    );
    let search = ApprovedTool {
        server_id: docs.id(),
        name: "search".to_owned(),
    };
    let tools = [
        search.clone(),
        ApprovedTool {
            server_id: gone.id(),
            name: "fetch".to_owned(),
        },
        ApprovedTool {
            server_id: theirs.id(),
            name: "read".to_owned(),
        },
    ];
    ChatRepository::approve(&store, owner, created.id(), &tools)
        .await
        .unwrap();
    ChatRepository::approve(&store, owner, created.id(), &tools)
        .await
        .unwrap();
    let stored = ChatRepository::get(&store, owner, created.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.title(), "Renamed", "nothing but approvals written");
    assert_eq!(
        stored.approved_tools(),
        [search],
        "once each; a deleted or foreign server is skipped"
    );

    let foreign = chat(&store, other).await;
    ChatRepository::approve(&store, owner, foreign.id(), &tools)
        .await
        .unwrap();
    let untouched = ChatRepository::get(&store, other, foreign.id())
        .await
        .unwrap()
        .unwrap();
    assert!(
        untouched.approved_tools().is_empty(),
        "another user's chat gets nothing"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn decide_applies_each_decision_once(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let model = model(&store, owner).await;
    let chat = chat(&store, owner).await;
    let docs = server(&store, owner, "docs").await;
    let call = |name: &str| ToolCallPart {
        id: CallId::generate(),
        provider_call_id: Some(format!("call_{name}")),
        server_id: Some(docs.id()),
        tool_name: name.to_owned(),
        alias: ToolAlias::restore(format!("docs__{name}")),
        arguments_raw: "{}".to_owned(),
        arguments: Some(json!({})),
        complete: true,
        approval: Approval::Pending,
        deny_reason: None,
    };
    let (search, delete) = (call("search"), call("delete"));
    let paused = Message::restore(MessageParts {
        status: MessageStatus::AwaitingApproval,
        finish_reason: Some(FinishReason::ToolCalls),
        parts: vec![
            Part::ToolCall(search.clone()),
            Part::ToolCall(delete.clone()),
        ],
        ..Message::assistant(owner, chat.id(), model.id(), model.provider_id(), at(1)).to_parts()
    });
    store.append(owner, &paused).await.unwrap();
    let decisions = [
        CallDecision {
            call_id: search.id,
            decision: Decision::Approve,
            scope: DecisionScope::Once,
            reason: None,
        },
        CallDecision {
            call_id: delete.id,
            decision: Decision::Deny,
            scope: DecisionScope::Once,
            reason: Some("Not now.".to_owned()),
        },
    ];

    assert!(
        !store.decide(other, paused.id(), &decisions).await.unwrap(),
        "another user's message is never decided"
    );
    // Two decisions at once: exactly one applies.
    let (first, second) = tokio::join!(
        store.decide(owner, paused.id(), &decisions),
        store.decide(owner, paused.id(), &decisions),
    );
    assert_eq!(
        [first.unwrap(), second.unwrap()]
            .iter()
            .filter(|applied| **applied)
            .count(),
        1
    );
    let history = store.history(owner, chat.id()).await.unwrap();
    let decided = &history[0];
    assert_eq!(decided.status(), MessageStatus::Complete);
    assert_eq!(
        *decided,
        paused.clone().decided(&decisions, decided.updated_at())
    );
    let approvals: Vec<(Approval, Option<&str>)> = decided
        .calls()
        .map(|call| (call.approval, call.deny_reason.as_deref()))
        .collect();
    assert_eq!(
        approvals,
        [
            (Approval::Approved, None),
            (Approval::Denied, Some("Not now."))
        ]
    );
    assert_eq!(
        updated_at(&store, owner, chat.id()).await,
        decided.updated_at(),
        "the chat moves with the decision"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn recover_marks_streaming_messages_interrupted(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let model = model(&store, owner).await;
    let chat = chat(&store, owner).await;
    let question = Message::user(
        owner,
        chat.id(),
        vec![Part::Text {
            text: "Hi".to_owned(),
        }],
        at(1),
    );
    store.append(owner, &question).await.unwrap();
    let streaming = Message::assistant(owner, chat.id(), model.id(), model.provider_id(), at(2));
    store.append(owner, &streaming).await.unwrap();
    let paused = Message::restore(MessageParts {
        status: MessageStatus::AwaitingApproval,
        ..Message::assistant(owner, chat.id(), model.id(), model.provider_id(), at(3)).to_parts()
    });
    store.append(owner, &paused).await.unwrap();

    assert_eq!(store.recover(at(9)).await.unwrap(), 1);
    let history = store.history(owner, chat.id()).await.unwrap();
    assert_eq!(history[1].status(), MessageStatus::Error);
    assert_eq!(history[1].error(), Some(copilot_core::INTERRUPTED));
    assert_eq!(history[1].updated_at(), at(9));
    assert_eq!(
        history[2].status(),
        MessageStatus::AwaitingApproval,
        "a paused answer stays valid"
    );
    assert_eq!(
        updated_at(&store, owner, chat.id()).await,
        at(3),
        "the chat does not move or turn unread"
    );
    assert_eq!(store.recover(at(10)).await.unwrap(), 0, "nothing left");
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
    let created: DateTime<Utc> =
        sqlx::query_scalar("SELECT created_at FROM turn_contexts WHERE chat_id = $1")
            .bind(chat.id().uuid())
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(created, at(NOW), "from the store's clock");
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

/// A user message with `text` in `chat`, written at `at(seconds)`.
async fn say(store: &PgStore, owner: UserId, chat: ChatId, text: &str, seconds: i64) -> Message {
    let message = Message::user(
        owner,
        chat,
        vec![Part::Text {
            text: text.to_owned(),
        }],
        at(seconds),
    );
    store.append(owner, &message).await.unwrap();
    message
}

/// The ids of a page, in order.
fn ids(page: &copilot_core::Page<copilot_core::ChatSummary>) -> Vec<ChatId> {
    page.items.iter().map(|row| row.chat.id()).collect()
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn the_chat_list_pages_by_update_time_and_filters(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let model = model(&store, owner).await;
    let project = Project::create(owner, "Research", at(0)).unwrap();
    ProjectRepository::insert(&store, &project).await.unwrap();
    let daily = ScheduleId::new(schedule(&store, owner, model.id().uuid(), true).await);

    // Oldest first: standalone, pinned in the project, a schedule run, and two chats written at
    // the same instant (the id breaks the tie).
    let plain = chat(&store, owner).await;
    say(&store, owner, plain.id(), "first", 1).await;
    let pinned = Chat::create(owner, at(0))
        .with_project(Some(project.id()))
        .with_pinned(true);
    ChatRepository::insert(&store, &pinned).await.unwrap();
    say(&store, owner, pinned.id(), "second", 2).await;
    let run = Chat::scheduled(owner, daily, "Daily · 1 Jan".to_owned(), at(3));
    ChatRepository::insert(&store, &run).await.unwrap();
    let tied = [chat(&store, owner).await, chat(&store, owner).await];
    for chat in &tied {
        say(&store, owner, chat.id(), "tied", 4).await;
    }
    let foreign = chat(&store, other).await;
    say(&store, other, foreign.id(), "theirs", 9).await;

    let mut newest_first: Vec<ChatId> = tied.iter().map(Chat::id).collect();
    newest_first.sort_by_key(|id| std::cmp::Reverse(id.uuid()));
    newest_first.extend([run.id(), pinned.id(), plain.id()]);

    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page = ChatRepository::list(
            &store,
            owner,
            ChatQuery {
                cursor,
                limit: 2,
                ..ChatQuery::default()
            },
        )
        .await
        .unwrap();
        assert!(page.items.len() <= 2);
        seen.extend(ids(&page));
        match page.next {
            Some(next) => {
                let last = page.items.last().unwrap();
                assert_eq!(
                    next,
                    ChatCursor {
                        updated_at: last.chat.updated_at(),
                        id: last.chat.id(),
                    }
                );
                cursor = Some(next);
            }
            None => break,
        }
    }
    assert_eq!(seen, newest_first, "every chat once, newest first");

    let exact = ChatRepository::list(
        &store,
        owner,
        ChatQuery {
            limit: 5,
            ..ChatQuery::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(exact.items.len(), 5);
    assert_eq!(exact.next, None, "a full last page has no next");

    let filtered = |query: ChatQuery| {
        let store = store.clone();
        async move { ids(&ChatRepository::list(&store, owner, query).await.unwrap()) }
    };
    assert_eq!(
        filtered(ChatQuery {
            project: Some(project.id()),
            ..ChatQuery::default()
        })
        .await,
        vec![pinned.id()]
    );
    assert_eq!(
        filtered(ChatQuery {
            pinned: Some(true),
            ..ChatQuery::default()
        })
        .await,
        vec![pinned.id()]
    );
    assert_eq!(
        filtered(ChatQuery {
            schedule: Some(daily),
            ..ChatQuery::default()
        })
        .await,
        vec![run.id()]
    );
    assert_eq!(
        filtered(ChatQuery {
            scheduled: Some(true),
            ..ChatQuery::default()
        })
        .await,
        vec![run.id()]
    );
    let recent = filtered(ChatQuery {
        scheduled: Some(false),
        pinned: Some(false),
        ..ChatQuery::default()
    })
    .await;
    assert_eq!(recent.len(), 3, "the filters combine: {recent:?}");
    assert!(!recent.contains(&run.id()) && !recent.contains(&pinned.id()));
    assert!(
        filtered(ChatQuery {
            project: Some(project.id()),
            scheduled: Some(true),
            ..ChatQuery::default()
        })
        .await
        .is_empty()
    );
    assert_eq!(
        ids(&ChatRepository::list(&store, other, ChatQuery::default())
            .await
            .unwrap()),
        vec![foreign.id()],
        "only the caller's chats"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn list_rows_flag_unread_chats_and_pending_approvals(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let model = model(&store, owner).await;
    let row = |chat: ChatId| {
        let store = store.clone();
        async move {
            ChatRepository::list(&store, owner, ChatQuery::default())
                .await
                .unwrap()
                .items
                .into_iter()
                .find(|row| row.chat.id() == chat)
                .expect("listed")
        }
    };

    let opened = chat(&store, owner).await;
    let flags = row(opened.id()).await;
    assert!(!flags.unread, "a new chat is read at creation");
    assert!(!flags.awaiting_approval);
    say(&store, owner, opened.id(), "Hello", 5).await;
    assert!(row(opened.id()).await.unread, "changed since it was opened");
    ChatRepository::mark_read(&store, owner, opened.id(), at(5))
        .await
        .unwrap();
    assert!(!row(opened.id()).await.unread, "read at the last change");

    let unopened = Chat::restore(copilot_core::ChatParts {
        last_read_at: None,
        ..Chat::create(owner, at(1)).to_parts()
    });
    ChatRepository::insert(&store, &unopened).await.unwrap();
    assert!(row(unopened.id()).await.unread, "never opened");

    let waiting = Message::restore(MessageParts {
        status: MessageStatus::AwaitingApproval,
        finish_reason: Some(FinishReason::ToolCalls),
        ..Message::assistant(owner, opened.id(), model.id(), model.provider_id(), at(6)).to_parts()
    });
    store.append(owner, &waiting).await.unwrap();
    assert!(row(opened.id()).await.awaiting_approval);
    let tool = Message::tool(owner, opened.id(), Vec::new(), at(7));
    store.append(owner, &tool).await.unwrap();
    assert!(
        row(opened.id()).await.awaiting_approval,
        "a tool message is not an answer"
    );
    let answered = Message::restore(MessageParts {
        status: MessageStatus::Complete,
        ..Message::assistant(owner, opened.id(), model.id(), model.provider_id(), at(8)).to_parts()
    });
    store.append(owner, &answered).await.unwrap();
    assert!(
        !row(opened.id()).await.awaiting_approval,
        "only the latest answer counts"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn tail_changes_delete_later_messages_in_one_chat(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let neighbour = chat(&store, owner).await;
    let chat = chat(&store, owner).await;
    let first = say(&store, owner, chat.id(), "one", 1).await;
    let second = say(&store, owner, chat.id(), "two", 2).await;
    say(&store, owner, chat.id(), "three", 3).await;
    let kept = say(&store, owner, neighbour.id(), "elsewhere", 4).await;

    assert_eq!(
        store
            .delete_after(other, chat.id(), first.id())
            .await
            .unwrap(),
        0,
        "another user's call deletes nothing"
    );
    assert_eq!(
        store
            .delete_after(owner, chat.id(), kept.id())
            .await
            .unwrap(),
        0,
        "a message of another chat matches nothing"
    );
    assert_eq!(
        store
            .delete_after(owner, chat.id(), second.id())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        store.history(owner, chat.id()).await.unwrap(),
        vec![first.clone(), second.clone()]
    );
    assert_eq!(
        updated_at(&store, owner, chat.id()).await,
        at(NOW),
        "a delete moves the chat on the store's clock"
    );

    let edited = Message::user(
        owner,
        chat.id(),
        vec![Part::Text {
            text: "two, edited".to_owned(),
        }],
        at(NOW + 1),
    );
    store
        .replace_tail(owner, chat.id(), second.id(), &edited)
        .await
        .unwrap();
    assert_eq!(
        store.history(owner, chat.id()).await.unwrap(),
        vec![first.clone(), edited.clone()]
    );
    assert_eq!(updated_at(&store, owner, chat.id()).await, at(NOW + 1));
    assert_eq!(
        store.history(owner, neighbour.id()).await.unwrap(),
        vec![kept],
        "other chats keep their messages"
    );

    let foreign = Message::user(
        other,
        chat.id(),
        vec![Part::Text {
            text: "mine".to_owned(),
        }],
        at(NOW + 2),
    );
    assert!(
        store
            .replace_tail(other, chat.id(), first.id(), &foreign)
            .await
            .is_err(),
        "the chat belongs to someone else"
    );
    assert_eq!(
        store.history(owner, chat.id()).await.unwrap(),
        vec![first, edited],
        "the failed replace rolled back its delete"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn search_finds_title_and_message_words_by_prefix(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let notes = Chat::create(owner, at(0)).titled_by("Rust ownership notes");
    ChatRepository::insert(&store, &notes).await.unwrap();
    let borrow = say(
        &store,
        owner,
        notes.id(),
        "The borrow checker\nenforces   ownership rules.",
        1,
    )
    .await;
    let long_text = format!(
        "{} the rustacean mascot is a crab {}",
        "words before ".repeat(40),
        "and words after ".repeat(40)
    );
    let long = say(&store, owner, notes.id(), &long_text, 2).await;
    let shopping = Chat::create(owner, at(0)).titled_by("Groceries");
    ChatRepository::insert(&store, &shopping).await.unwrap();
    let bread = say(&store, owner, shopping.id(), "Buy rustic bread", 3).await;
    let foreign = Chat::create(other, at(0)).titled_by("Rust secrets");
    ChatRepository::insert(&store, &foreign).await.unwrap();
    say(&store, other, foreign.id(), "rust rust rust", 4).await;

    let hits = store.search(owner, "rust", 10).await.unwrap();
    assert_eq!(hits[0].chat_id, notes.id(), "the title first");
    assert_eq!(hits[0].message_id, None);
    assert_eq!(hits[0].snippet, "Rust ownership notes");
    let messages: Vec<_> = hits[1..].iter().map(|hit| hit.message_id).collect();
    assert_eq!(messages.len(), 2, "{hits:?}");
    assert!(messages.contains(&Some(long.id())) && messages.contains(&Some(bread.id())));
    assert!(
        hits.iter().all(|hit| hit.chat_id != foreign.id()),
        "only the caller's chats"
    );
    let crab = hits
        .iter()
        .find(|hit| hit.message_id == Some(long.id()))
        .unwrap();
    assert_eq!(crab.chat_title, "Rust ownership notes");
    assert!(
        crab.snippet.contains("rustacean mascot"),
        "{}",
        crab.snippet
    );
    assert!(crab.snippet.len() < 300, "the words around the match");

    let both = store.search(owner, "BORROW check", 10).await.unwrap();
    assert_eq!(both.len(), 1, "every word must match: {both:?}");
    assert_eq!(both[0].message_id, Some(borrow.id()));
    assert_eq!(
        both[0].snippet, "The borrow checker enforces ownership rules.",
        "one line"
    );
    let prefix = store.search(owner, "own", 10).await.unwrap();
    assert_eq!(prefix.len(), 2, "the title and one message: {prefix:?}");

    assert_eq!(store.search(owner, "rust", 1).await.unwrap().len(), 1);
    assert!(store.search(owner, "???", 10).await.unwrap().is_empty());
    assert!(store.search(owner, "crabs", 10).await.unwrap().is_empty());
    let syntax = store
        .search(owner, "rust & | ! <-> ( ' :*", 10)
        .await
        .unwrap();
    assert_eq!(syntax.len(), 3, "query syntax is plain text: {syntax:?}");
    assert!(
        store
            .search(other, "groceries", 10)
            .await
            .unwrap()
            .is_empty()
    );
}
