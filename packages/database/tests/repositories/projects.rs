use copilot_core::{
    BuiltinTool, Chat, ChatRepository, Project, ProjectId, ProjectParts, ProjectQuery,
    ProjectRepository, ReasoningLevel, ServerOverride, ToolServerId, UserId,
};
use copilot_database::{DbPool, PgStore};
use uuid::Uuid;

use crate::{at, file_in, model, server, store, user};

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn a_project_is_read_with_its_tool_servers_by_its_owner_only(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let default_model = model(&store, owner).await;
    let server = Uuid::now_v7();
    let project = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO tool_servers (created_at, updated_at, id, user_id, name, slug, url, \
         headers_sealed) VALUES ($1, $1, $2, $3, 'Docs', 'docs', 'https://mcp.example.com', NULL)",
    )
    .bind(at(0))
    .bind(server)
    .bind(owner.uuid())
    .execute(store.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO projects (created_at, updated_at, id, user_id, default_model_id, pinned, \
         name, icon, instructions, reasoning, builtin_tools) \
         VALUES ($1, $2, $3, $4, $5, true, 'Research', 'R', 'Cite sources.', 'high', \
         ARRAY['schedules'])",
    )
    .bind(at(0))
    .bind(at(5))
    .bind(project)
    .bind(owner.uuid())
    .bind(default_model.id().uuid())
    .execute(store.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO project_tool_servers (project_id, tool_server_id, user_id) \
         VALUES ($1, $2, $3)",
    )
    .bind(project)
    .bind(server)
    .bind(owner.uuid())
    .execute(store.pool())
    .await
    .unwrap();

    let read = ProjectRepository::get(&store, owner, ProjectId::new(project))
        .await
        .unwrap()
        .expect("the owner reads it");
    assert_eq!(read.id(), ProjectId::new(project));
    assert_eq!(read.user_id(), owner);
    assert_eq!(read.name(), "Research");
    assert_eq!(read.icon(), "R");
    assert_eq!(read.instructions(), "Cite sources.");
    assert_eq!(read.default_model_id(), Some(default_model.id()));
    assert_eq!(read.reasoning(), Some(ReasoningLevel::High));
    assert_eq!(read.builtin_tools(), [BuiltinTool::Schedules]);
    assert_eq!(read.tool_server_ids(), [ToolServerId::new(server)]);
    assert!(read.pinned());
    assert_eq!(read.created_at(), at(0));
    assert_eq!(read.updated_at(), at(5));

    assert_eq!(
        ProjectRepository::get(&store, other, ProjectId::new(project))
            .await
            .unwrap(),
        None,
        "another user's project reads as missing"
    );
    assert_eq!(
        ProjectRepository::get(&store, owner, ProjectId::generate())
            .await
            .unwrap(),
        None
    );
}

/// A project of `owner` with `servers`, saved through the repository.
async fn saved(
    store: &PgStore,
    owner: UserId,
    name: &str,
    updated: i64,
    servers: Vec<ToolServerId>,
) -> Project {
    let project = Project::create(owner, name, at(0))
        .unwrap()
        .with_tool_servers(servers)
        .touched(at(updated));
    ProjectRepository::insert(store, &project).await.unwrap();
    project
}

/// How many rows of `table` name the project in `column`.
async fn rows(store: &PgStore, table: &str, column: &str, project: ProjectId) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM {table} WHERE {column} = $1"
    )))
    .bind(project.uuid())
    .fetch_one(store.pool())
    .await
    .unwrap()
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn projects_are_inserted_listed_and_updated_with_their_tool_servers(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let default_model = model(&store, owner).await;
    let docs = server(&store, owner, "docs").await.id();
    let web = server(&store, owner, "web").await.id();
    let mail = server(&store, owner, "mail").await.id();

    let research = Project::create(owner, "Research", at(0))
        .unwrap()
        .with_icon("🔬")
        .unwrap()
        .with_instructions("Cite sources.")
        .with_default_model(Some(default_model.id()))
        .with_reasoning(Some(ReasoningLevel::High))
        .with_builtin_tools(vec![BuiltinTool::Schedules])
        .with_tool_servers(vec![docs, web])
        .with_pinned(true);
    ProjectRepository::insert(&store, &research).await.unwrap();
    let read = ProjectRepository::get(&store, owner, research.id())
        .await
        .unwrap()
        .expect("inserted");
    assert_eq!(read.name(), "Research");
    assert_eq!(read.icon(), "🔬");
    assert_eq!(read.instructions(), "Cite sources.");
    assert_eq!(read.default_model_id(), Some(default_model.id()));
    assert_eq!(read.reasoning(), Some(ReasoningLevel::High));
    assert_eq!(read.builtin_tools(), [BuiltinTool::Schedules]);
    assert!(read.pinned());
    let mut expected = vec![docs, web];
    expected.sort();
    assert_eq!(read.tool_server_ids(), expected.as_slice());

    let older = saved(&store, owner, "Older", 1, Vec::new()).await;
    let newer = saved(&store, owner, "Newer", 9, Vec::new()).await;
    saved(&store, other, "Theirs", 20, Vec::new()).await;
    let names = |projects: Vec<Project>| {
        projects
            .iter()
            .map(|project| project.name().to_owned())
            .collect::<Vec<_>>()
    };
    let all = ProjectRepository::list(&store, owner, ProjectQuery::default())
        .await
        .unwrap();
    assert_eq!(
        names(all),
        ["Newer", "Older", "Research"],
        "most recently updated first, the owner's only"
    );
    let pinned = ProjectQuery {
        pinned: Some(true),
        limit: None,
    };
    assert_eq!(
        names(
            ProjectRepository::list(&store, owner, pinned)
                .await
                .unwrap()
        ),
        ["Research"]
    );
    let unpinned = ProjectQuery {
        pinned: Some(false),
        limit: Some(1),
    };
    assert_eq!(
        names(
            ProjectRepository::list(&store, owner, unpinned)
                .await
                .unwrap()
        ),
        ["Newer"]
    );

    let changed = read
        .clone()
        .with_name("Renamed")
        .unwrap()
        .with_default_model(None)
        .with_reasoning(None)
        .with_builtin_tools(Vec::new())
        .with_tool_servers(vec![web, mail])
        .with_pinned(false)
        .touched(at(30));
    ProjectRepository::update(&store, &changed).await.unwrap();
    let read = ProjectRepository::get(&store, owner, research.id())
        .await
        .unwrap()
        .expect("updated");
    let mut expected = vec![web, mail];
    expected.sort();
    assert_eq!(read.tool_server_ids(), expected.as_slice(), "rows replaced");
    assert_eq!(read.name(), "Renamed");
    assert_eq!(read.default_model_id(), None);
    assert_eq!(read.reasoning(), None);
    assert!(read.builtin_tools().is_empty());
    assert!(!read.pinned());
    assert_eq!(read.created_at(), at(0));
    assert_eq!(read.updated_at(), at(30));
    assert_eq!(
        names(
            ProjectRepository::list(&store, owner, ProjectQuery::default())
                .await
                .unwrap()
        ),
        ["Renamed", "Newer", "Older"]
    );
    assert_eq!(older.user_id(), newer.user_id());

    let cleared = read.clone().with_tool_servers(Vec::new()).touched(at(31));
    ProjectRepository::update(&store, &cleared).await.unwrap();
    assert_eq!(
        rows(&store, "project_tool_servers", "project_id", research.id()).await,
        0
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn project_writes_never_reach_another_users_rows(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let theirs = server(&store, other, "theirs").await.id();
    let their_model = model(&store, other).await.id();

    let with_server = Project::create(owner, "Borrowed", at(0))
        .unwrap()
        .with_tool_servers(vec![theirs]);
    assert!(
        ProjectRepository::insert(&store, &with_server)
            .await
            .is_err(),
        "the composite key refuses another user's tool server"
    );
    assert_eq!(
        ProjectRepository::get(&store, owner, with_server.id())
            .await
            .unwrap(),
        None,
        "the project row rolled back with its tool server rows"
    );
    let with_model = Project::create(owner, "Borrowed", at(0))
        .unwrap()
        .with_default_model(Some(their_model));
    assert!(
        ProjectRepository::insert(&store, &with_model)
            .await
            .is_err()
    );

    let project = saved(&store, owner, "Mine", 1, Vec::new()).await;
    let forged = Project::restore(ProjectParts {
        id: project.id(),
        user_id: other,
        name: "Taken".to_owned(),
        icon: String::new(),
        instructions: String::new(),
        default_model_id: None,
        reasoning: None,
        builtin_tools: Vec::new(),
        tool_server_ids: vec![theirs],
        pinned: true,
        created_at: at(0),
        updated_at: at(5),
    });
    ProjectRepository::update(&store, &forged).await.unwrap();
    let read = ProjectRepository::get(&store, owner, project.id())
        .await
        .unwrap()
        .expect("still there");
    assert_eq!(read.name(), "Mine", "another user's update changes nothing");
    assert!(read.tool_server_ids().is_empty());
    assert!(
        !ProjectRepository::delete(&store, other, project.id())
            .await
            .unwrap(),
        "another user cannot delete it"
    );
    assert!(
        ProjectRepository::get(&store, owner, project.id())
            .await
            .unwrap()
            .is_some()
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn deleting_a_project_removes_its_chats_files_and_schedules(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let docs = server(&store, owner, "docs").await.id();
    let project = saved(&store, owner, "Doomed", 1, vec![docs]).await;
    let kept = saved(&store, owner, "Kept", 1, vec![docs]).await;
    for target in [project.id(), kept.id()] {
        let chat = Chat::create(owner, at(0)).with_project(Some(target));
        ChatRepository::insert(&store, &chat).await.unwrap();
        file_in(&store, owner, Some(target), None).await;
        sqlx::query(
            "INSERT INTO schedules (created_at, updated_at, id, user_id, project_id, enabled, \
             name, cron, timezone, prompt, created_by) \
             VALUES ($1, $1, $2, $3, $4, false, 'Daily', '0 9 * * *', 'UTC', 'Go.', 'user')",
        )
        .bind(at(0))
        .bind(Uuid::now_v7())
        .bind(owner.uuid())
        .bind(target.uuid())
        .execute(store.pool())
        .await
        .unwrap();
    }

    assert!(
        ProjectRepository::delete(&store, owner, project.id())
            .await
            .unwrap()
    );
    for (table, column) in [
        ("projects", "id"),
        ("project_tool_servers", "project_id"),
        ("chats", "project_id"),
        ("files", "project_id"),
        ("schedules", "project_id"),
    ] {
        assert_eq!(
            rows(&store, table, column, project.id()).await,
            0,
            "{table}"
        );
        assert_eq!(rows(&store, table, column, kept.id()).await, 1, "{table}");
    }
    assert!(
        !ProjectRepository::delete(&store, owner, project.id())
            .await
            .unwrap(),
        "already gone"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn saving_a_chat_as_a_project_moves_the_chat_and_its_attachments(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let docs = server(&store, owner, "docs").await.id();
    let chat = Chat::create(owner, at(0))
        .with_instructions("Be brief.")
        .with_tool_overrides(vec![ServerOverride {
            server_id: docs,
            enabled: true,
        }]);
    ChatRepository::insert(&store, &chat).await.unwrap();
    let attachment = file_in(&store, owner, None, Some(chat.id())).await;

    let project = Project::create(owner, "Saved", at(7))
        .unwrap()
        .with_instructions("Be brief.")
        .with_tool_servers(vec![docs]);
    assert!(store.insert_with_chat(&project, chat.id()).await.unwrap());

    let read = ProjectRepository::get(&store, owner, project.id())
        .await
        .unwrap()
        .expect("inserted");
    assert_eq!(read.tool_server_ids(), [docs]);
    let moved = ChatRepository::get(&store, owner, chat.id())
        .await
        .unwrap()
        .expect("the chat stays");
    assert_eq!(moved.project_id(), Some(project.id()));
    assert_eq!(moved.instructions(), "");
    assert!(moved.tool_overrides().is_empty());
    assert_eq!(
        moved.updated_at(),
        chat.updated_at(),
        "settings never move it"
    );
    let (project_id, chat_id, included): (Option<Uuid>, Option<Uuid>, bool) =
        sqlx::query_as("SELECT project_id, chat_id, included FROM files WHERE id = $1")
            .bind(attachment)
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(project_id, Some(project.id().uuid()));
    assert_eq!(chat_id, None);
    assert!(included, "an attachment becomes an included project file");

    let theirs = Chat::create(other, at(0));
    ChatRepository::insert(&store, &theirs).await.unwrap();
    let stolen = Project::create(owner, "Stolen", at(8)).unwrap();
    assert!(!store.insert_with_chat(&stolen, theirs.id()).await.unwrap());
    assert_eq!(
        ProjectRepository::get(&store, owner, stolen.id())
            .await
            .unwrap(),
        None,
        "the project rolls back when the chat is not the owner's"
    );
    let untouched = ChatRepository::get(&store, other, theirs.id())
        .await
        .unwrap()
        .expect("their chat");
    assert_eq!(untouched.project_id(), None);
}

/// Waits until `count` sessions of the test's database wait for a lock.
async fn lock_waiters(store: &PgStore, count: i64) {
    for _ in 0..250 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(store.pool())
        .await
        .unwrap();
        if waiting >= count {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("{count} sessions never waited for the lock");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn two_saves_of_one_chat_at_once_create_one_project(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let chat = Chat::create(owner, at(0)).with_instructions("Be brief.");
    ChatRepository::insert(&store, &chat).await.unwrap();
    let attachment = file_in(&store, owner, None, Some(chat.id())).await;
    let first = Project::create(owner, "First", at(1)).unwrap();
    let second = Project::create(owner, "Second", at(2)).unwrap();

    // Holds the chat's row until both saves have inserted their project and wait to move the
    // chat, as a double-click does.
    let mut holder = store.pool().begin().await.unwrap();
    sqlx::query("SELECT 1 FROM chats WHERE id = $1 FOR UPDATE")
        .bind(chat.id().uuid())
        .execute(&mut *holder)
        .await
        .unwrap();
    let (saved_first, saved_second, ()) = tokio::join!(
        store.insert_with_chat(&first, chat.id()),
        store.insert_with_chat(&second, chat.id()),
        async {
            lock_waiters(&store, 2).await;
            holder.commit().await.unwrap();
        },
    );
    let saved = [saved_first.unwrap(), saved_second.unwrap()];
    assert_eq!(saved.iter().filter(|saved| **saved).count(), 1, "{saved:?}");
    let winner = if saved[0] { first.id() } else { second.id() };

    let projects = ProjectRepository::list(&store, owner, ProjectQuery::default())
        .await
        .unwrap();
    let ids: Vec<ProjectId> = projects.iter().map(Project::id).collect();
    assert_eq!(ids, [winner], "the other save rolled its project back");
    let moved = ChatRepository::get(&store, owner, chat.id())
        .await
        .unwrap()
        .expect("the chat stays");
    assert_eq!(moved.project_id(), Some(winner));
    let files: Option<Uuid> = sqlx::query_scalar("SELECT project_id FROM files WHERE id = $1")
        .bind(attachment)
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(files, Some(winner.uuid()), "the files went with the chat");
    assert!(
        !store
            .insert_with_chat(&Project::create(owner, "Third", at(3)).unwrap(), chat.id())
            .await
            .unwrap(),
        "a chat in a project is not saved again"
    );
}
