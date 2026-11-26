//! What deleting a row removes or clears (docs/design/copilot.md, Tables): deleting an account
//! in `auth.users` removes all of the user's Copilot rows in one statement, references to
//! deleted models, providers, schedules, and chats become null, and owned rows cascade.

use copilot_database::DbPool;
use uuid::Uuid;

use crate::fixtures::{self, Graph, TABLES};

/// Deletes the row of `table` with `id`, and checks it existed.
async fn delete(pool: &DbPool, table: &str, id: Uuid) {
    let sql = format!("DELETE FROM {table} WHERE id = $1");
    let deleted = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .execute(pool)
        .await
        .expect("row deleted")
        .rows_affected();
    assert_eq!(deleted, 1, "{table} {id}");
}

/// `column` of the row of `table` with `id`; `None` when the column is null. Fails when the row
/// is gone.
async fn reference(pool: &DbPool, table: &str, column: &str, id: Uuid) -> Option<Uuid> {
    let sql = format!("SELECT {column} FROM {table} WHERE id = $1");
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("row still exists")
}

async fn exists(pool: &DbPool, table: &str, id: Uuid) -> bool {
    fixtures::count(pool, table, "id = $1", id).await == 1
}

/// Rows of every owner-scoped table that belong to `user`, plus the file bytes of their files.
async fn rows(pool: &DbPool, user: Uuid) -> Vec<(&'static str, i64)> {
    let mut counts = Vec::new();
    for table in TABLES {
        let filter = if *table == "file_contents" {
            "file_id IN (SELECT id FROM files WHERE user_id = $1)"
        } else {
            "user_id = $1"
        };
        counts.push((*table, fixtures::count(pool, table, filter, user).await));
    }
    counts
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn deleting_an_account_removes_every_row_of_the_user(pool: DbPool) {
    fixtures::migrated(&pool).await;
    let first = fixtures::user(&pool).await;
    let second = fixtures::user(&pool).await;
    Graph::insert(&pool, first).await;
    Graph::insert(&pool, second).await;
    let kept = rows(&pool, second).await;
    assert!(
        rows(&pool, first).await.iter().all(|(_, count)| *count > 0),
        "the fixture fills every table"
    );

    // What lib-rustauth's `DELETE /admin/users/{id}` runs: a plain delete that only succeeds
    // when every reference cascades or sets null.
    sqlx::query("DELETE FROM auth.users WHERE id = $1")
        .bind(first)
        .execute(&pool)
        .await
        .expect("account deleted without a blocking reference");

    for (table, count) in rows(&pool, first).await {
        assert_eq!(count, 0, "{table} keeps rows of a deleted account");
    }
    assert_eq!(
        rows(&pool, second).await,
        kept,
        "other users keep their rows"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn deleting_a_model_clears_references_to_it(pool: DbPool) {
    fixtures::migrated(&pool).await;
    let user = fixtures::user(&pool).await;
    let g = Graph::insert(&pool, user).await;

    delete(&pool, "models", g.model).await;

    assert_eq!(
        reference(&pool, "projects", "default_model_id", g.project).await,
        None
    );
    assert_eq!(
        reference(&pool, "schedules", "model_id", g.schedule).await,
        None
    );
    assert_eq!(reference(&pool, "chats", "model_id", g.chat).await, None);
    assert_eq!(
        reference(&pool, "chats", "model_id", g.standalone).await,
        None
    );
    assert_eq!(
        reference(&pool, "messages", "model_id", g.reply).await,
        None
    );
    assert_eq!(
        reference(&pool, "messages", "provider_id", g.reply).await,
        Some(g.provider)
    );
    for (table, id) in [
        ("projects", g.project),
        ("schedules", g.schedule),
        ("chats", g.chat),
        ("messages", g.reply),
    ] {
        assert_eq!(
            reference(&pool, table, "user_id", id).await,
            Some(user),
            "{table}"
        );
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn deleting_a_provider_removes_its_models_and_keeps_messages(pool: DbPool) {
    fixtures::migrated(&pool).await;
    let user = fixtures::user(&pool).await;
    let g = Graph::insert(&pool, user).await;

    delete(&pool, "providers", g.provider).await;

    assert!(!exists(&pool, "models", g.model).await);
    assert_eq!(
        reference(&pool, "messages", "provider_id", g.reply).await,
        None
    );
    assert_eq!(
        reference(&pool, "messages", "model_id", g.reply).await,
        None
    );
    assert_eq!(reference(&pool, "chats", "model_id", g.chat).await, None);
    assert_eq!(
        reference(&pool, "schedules", "model_id", g.schedule).await,
        None
    );
    assert_eq!(
        reference(&pool, "projects", "default_model_id", g.project).await,
        None
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn deleting_a_schedule_keeps_the_chats_it_started(pool: DbPool) {
    fixtures::migrated(&pool).await;
    let user = fixtures::user(&pool).await;
    let g = Graph::insert(&pool, user).await;

    delete(&pool, "schedules", g.schedule).await;

    assert_eq!(reference(&pool, "chats", "schedule_id", g.chat).await, None);
    assert_eq!(
        fixtures::count(&pool, "messages", "chat_id = $1", g.chat).await,
        2
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn deleting_a_chat_removes_its_rows_and_keeps_its_schedules(pool: DbPool) {
    fixtures::migrated(&pool).await;
    let user = fixtures::user(&pool).await;
    let g = Graph::insert(&pool, user).await;

    // The chat that created the schedule: the schedule stays and forgets it.
    delete(&pool, "chats", g.standalone).await;
    assert_eq!(
        reference(&pool, "schedules", "source_chat_id", g.schedule).await,
        None
    );
    assert_eq!(
        fixtures::count(&pool, "chat_tool_servers", "chat_id = $1", g.standalone).await,
        0
    );

    // The chat with messages, a snapshot, an approval, and a file.
    delete(&pool, "chats", g.chat).await;
    for table in ["messages", "turn_contexts", "chat_tool_approvals", "files"] {
        assert_eq!(
            fixtures::count(&pool, table, "chat_id = $1", g.chat).await,
            0,
            "{table}"
        );
    }
    assert_eq!(
        fixtures::count(&pool, "file_contents", "file_id = $1", g.chat_file).await,
        0
    );
    assert!(exists(&pool, "schedules", g.schedule).await);
    assert!(exists(&pool, "files", g.project_file).await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn deleting_a_project_removes_its_chats_files_and_schedules(pool: DbPool) {
    fixtures::migrated(&pool).await;
    let user = fixtures::user(&pool).await;
    let g = Graph::insert(&pool, user).await;

    delete(&pool, "projects", g.project).await;

    assert!(!exists(&pool, "chats", g.chat).await);
    assert!(!exists(&pool, "schedules", g.schedule).await);
    assert!(!exists(&pool, "files", g.project_file).await);
    assert!(
        !exists(&pool, "files", g.chat_file).await,
        "files of the project's chats"
    );
    assert_eq!(
        fixtures::count(&pool, "project_tool_servers", "project_id = $1", g.project).await,
        0
    );
    assert_eq!(
        fixtures::count(&pool, "messages", "chat_id = $1", g.chat).await,
        0
    );
    for file in [g.project_file, g.chat_file] {
        assert_eq!(
            fixtures::count(&pool, "file_contents", "file_id = $1", file).await,
            0
        );
    }
    // A standalone chat stays, with its tool servers.
    assert!(exists(&pool, "chats", g.standalone).await);
    assert_eq!(
        fixtures::count(&pool, "chat_tool_servers", "chat_id = $1", g.standalone).await,
        1
    );
    assert!(exists(&pool, "tool_servers", g.tool_server).await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn deleting_a_tool_server_removes_its_selections(pool: DbPool) {
    fixtures::migrated(&pool).await;
    let user = fixtures::user(&pool).await;
    let g = Graph::insert(&pool, user).await;

    delete(&pool, "tool_servers", g.tool_server).await;

    for table in [
        "project_tool_servers",
        "chat_tool_servers",
        "chat_tool_approvals",
    ] {
        assert_eq!(
            fixtures::count(&pool, table, "tool_server_id = $1", g.tool_server).await,
            0,
            "{table}"
        );
    }
    assert!(exists(&pool, "projects", g.project).await);
    assert!(exists(&pool, "chats", g.chat).await);
    assert!(exists(&pool, "chats", g.standalone).await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn deleting_a_file_removes_its_bytes(pool: DbPool) {
    fixtures::migrated(&pool).await;
    let user = fixtures::user(&pool).await;
    let g = Graph::insert(&pool, user).await;

    delete(&pool, "files", g.project_file).await;

    assert_eq!(
        fixtures::count(&pool, "file_contents", "file_id = $1", g.project_file).await,
        0
    );
    assert_eq!(
        fixtures::count(&pool, "file_contents", "file_id = $1", g.chat_file).await,
        1
    );
}
