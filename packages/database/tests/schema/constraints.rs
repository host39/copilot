//! Tenant isolation keys, `CHECK` constraints, and column defaults (docs/design/copilot.md,
//! Data Model).

use copilot_database::DbPool;
use uuid::Uuid;

use crate::fixtures::{self, CHECK_VIOLATION, FOREIGN_KEY_VIOLATION, Graph, TABLES};

/// Tables that reference `auth.users (id)` directly. Every other owner-scoped table reaches it
/// through a composite key to one of these.
const OWNERS: &[&str] = &[
    "chats",
    "projects",
    "providers",
    "schedules",
    "tool_servers",
    "user_settings",
];

/// A foreign key of a `public` table: its table, columns, referenced table (schema-qualified
/// when not `public`), referenced columns, delete action, and the columns `SET NULL` clears.
#[derive(Debug, sqlx::FromRow)]
struct ForeignKey {
    table: String,
    columns: Vec<String>,
    target: String,
    referenced: Vec<String>,
    action: String,
    cleared: Vec<String>,
}

async fn foreign_keys(pool: &DbPool) -> Vec<ForeignKey> {
    sqlx::query_as(
        "SELECT src.relname::text AS table, \
           ARRAY(SELECT a.attname::text FROM unnest(c.conkey) WITH ORDINALITY k(num, pos) \
             JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.num \
             ORDER BY k.pos) AS columns, \
           CASE WHEN dst_ns.nspname = 'public' THEN dst.relname::text \
             ELSE dst_ns.nspname || '.' || dst.relname END AS target, \
           ARRAY(SELECT a.attname::text FROM unnest(c.confkey) WITH ORDINALITY k(num, pos) \
             JOIN pg_attribute a ON a.attrelid = c.confrelid AND a.attnum = k.num \
             ORDER BY k.pos) AS referenced, \
           c.confdeltype::text AS action, \
           ARRAY(SELECT a.attname::text FROM unnest(c.confdelsetcols) AS k(num) \
             JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.num) AS cleared \
         FROM pg_constraint c \
         JOIN pg_class src ON src.oid = c.conrelid \
         JOIN pg_namespace src_ns ON src_ns.oid = src.relnamespace \
         JOIN pg_class dst ON dst.oid = c.confrelid \
         JOIN pg_namespace dst_ns ON dst_ns.oid = dst.relnamespace \
         WHERE c.contype = 'f' AND src_ns.nspname = 'public' \
         ORDER BY src.relname, c.conname",
    )
    .fetch_all(pool)
    .await
    .expect("foreign keys listed")
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn owner_scoped_tables_carry_a_user_id(pool: DbPool) {
    fixtures::migrated(&pool).await;
    let owned: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.columns \
         WHERE table_schema = 'public' AND column_name = 'user_id' AND is_nullable = 'NO' \
           AND data_type = 'uuid' ORDER BY table_name",
    )
    .fetch_all(&pool)
    .await
    .expect("columns listed");
    let expected: Vec<&str> = TABLES
        .iter()
        .copied()
        .filter(|table| *table != "file_contents")
        .collect();
    assert_eq!(owned, expected);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn references_keep_rows_with_their_owner(pool: DbPool) {
    fixtures::migrated(&pool).await;
    let keys = foreign_keys(&pool).await;
    assert!(!keys.is_empty());

    let mut owners = Vec::new();
    for key in &keys {
        match key.target.as_str() {
            "auth.users" => {
                assert_eq!(key.columns, ["user_id"], "{key:?}");
                assert_eq!(key.referenced, ["id"], "{key:?}");
                assert_eq!(key.action, "c", "deleting an account cascades: {key:?}");
                owners.push(key.table.as_str());
            }
            "files" => {
                assert_eq!(key.table, "file_contents", "{key:?}");
                assert_eq!(key.action, "c", "{key:?}");
            }
            _ => {
                assert!(
                    !key.target.contains('.'),
                    "only auth.users is outside public"
                );
                assert_eq!(key.columns.len(), 2, "{key:?}");
                assert_eq!(
                    key.columns[1], "user_id",
                    "composite with the owner: {key:?}"
                );
                assert_eq!(key.referenced, ["id", "user_id"], "{key:?}");
                match key.action.as_str() {
                    "c" => assert!(key.cleared.is_empty(), "{key:?}"),
                    "n" => assert_eq!(
                        key.cleared,
                        [key.columns[0].clone()],
                        "SET NULL clears the reference, never user_id: {key:?}"
                    ),
                    _ => panic!("every reference cascades or sets null: {key:?}"),
                }
            }
        }
    }
    owners.sort_unstable();
    assert_eq!(owners, OWNERS);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn rows_cannot_reference_another_users_rows(pool: DbPool) {
    fixtures::migrated(&pool).await;
    let first = fixtures::user(&pool).await;
    let second = fixtures::user(&pool).await;
    let a = Graph::insert(&pool, first).await;
    let b = Graph::insert(&pool, second).await;

    let statements = [
        format!(
            "UPDATE models SET provider_id = '{}', remote_id = 'other' WHERE id = '{}'",
            a.provider, b.model
        ),
        format!(
            "UPDATE projects SET default_model_id = '{}' WHERE id = '{}'",
            a.model, b.project
        ),
        format!(
            "UPDATE project_tool_servers SET tool_server_id = '{}' WHERE project_id = '{}'",
            a.tool_server, b.project
        ),
        format!(
            "UPDATE schedules SET project_id = '{}' WHERE id = '{}'",
            a.project, b.schedule
        ),
        format!(
            "UPDATE schedules SET model_id = '{}' WHERE id = '{}'",
            a.model, b.schedule
        ),
        format!(
            "UPDATE schedules SET source_chat_id = '{}' WHERE id = '{}'",
            a.chat, b.schedule
        ),
        format!(
            "UPDATE chats SET project_id = '{}' WHERE id = '{}'",
            a.project, b.chat
        ),
        format!(
            "UPDATE chats SET model_id = '{}' WHERE id = '{}'",
            a.model, b.chat
        ),
        format!(
            "UPDATE chats SET schedule_id = '{}' WHERE id = '{}'",
            a.schedule, b.chat
        ),
        format!(
            "UPDATE chat_tool_servers SET tool_server_id = '{}' WHERE chat_id = '{}'",
            a.tool_server, b.standalone
        ),
        format!(
            "UPDATE chat_tool_approvals SET tool_server_id = '{}' WHERE chat_id = '{}'",
            a.tool_server, b.chat
        ),
        format!(
            "UPDATE messages SET chat_id = '{}' WHERE id = '{}'",
            a.chat, b.reply
        ),
        format!(
            "UPDATE messages SET model_id = '{}' WHERE id = '{}'",
            a.model, b.reply
        ),
        format!(
            "UPDATE messages SET provider_id = '{}' WHERE id = '{}'",
            a.provider, b.reply
        ),
        format!(
            "UPDATE turn_contexts SET chat_id = '{}', hash = decode(repeat('08', 32), 'hex') \
             WHERE chat_id = '{}'",
            a.chat, b.chat
        ),
        format!(
            "UPDATE files SET project_id = '{}' WHERE id = '{}'",
            a.project, b.project_file
        ),
        format!(
            "UPDATE files SET chat_id = '{}' WHERE id = '{}'",
            a.chat, b.chat_file
        ),
    ];
    for statement in &statements {
        assert_eq!(
            fixtures::failure(&pool, statement).await.as_deref(),
            Some(FOREIGN_KEY_VIOLATION),
            "{statement}"
        );
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn checks_reject_values_core_cannot_read(pool: DbPool) {
    fixtures::migrated(&pool).await;
    let user = fixtures::user(&pool).await;
    let g = Graph::insert(&pool, user).await;
    let user_message = format!("role = 'user' AND chat_id = '{}'", g.chat);

    let rejected = [
        format!(
            "UPDATE providers SET kind = 'gemini' WHERE id = '{}'",
            g.provider
        ),
        format!(
            "UPDATE providers SET preset = 'azure' WHERE id = '{}'",
            g.provider
        ),
        format!(
            "UPDATE providers SET api = 'completions' WHERE id = '{}'",
            g.provider
        ),
        format!(
            "UPDATE providers SET api = NULL WHERE id = '{}'",
            g.provider
        ),
        format!(
            "UPDATE providers SET kind = 'anthropic', preset = 'custom' WHERE id = '{}'",
            g.provider
        ),
        format!(
            "UPDATE models SET context_window = -1 WHERE id = '{}'",
            g.model
        ),
        format!(
            "UPDATE models SET max_output_tokens = -1 WHERE id = '{}'",
            g.model
        ),
        format!(
            "UPDATE tool_servers SET slug = 'Docs' WHERE id = '{}'",
            g.tool_server
        ),
        format!(
            "UPDATE tool_servers SET slug = '' WHERE id = '{}'",
            g.tool_server
        ),
        format!(
            "UPDATE tool_servers SET slug = 'abcdefghijklm' WHERE id = '{}'",
            g.tool_server
        ),
        format!(
            "UPDATE tool_servers SET slug = 'my_docs' WHERE id = '{}'",
            g.tool_server
        ),
        format!(
            "UPDATE tool_servers SET approval = 'never' WHERE id = '{}'",
            g.tool_server
        ),
        format!(
            "UPDATE tool_servers SET tools_cached = '{{}}' WHERE id = '{}'",
            g.tool_server
        ),
        format!(
            "UPDATE tool_servers SET tools_fetched_at = now() WHERE id = '{}'",
            g.tool_server
        ),
        format!(
            "UPDATE projects SET reasoning = 'max' WHERE id = '{}'",
            g.project
        ),
        format!(
            "UPDATE projects SET builtin_tools = '{{web}}' WHERE id = '{}'",
            g.project
        ),
        format!(
            "UPDATE projects SET builtin_tools = '{{schedules,web}}' WHERE id = '{}'",
            g.project
        ),
        format!(
            "UPDATE schedules SET reasoning = 'max' WHERE id = '{}'",
            g.schedule
        ),
        format!(
            "UPDATE schedules SET on_ask = 'ask' WHERE id = '{}'",
            g.schedule
        ),
        format!(
            "UPDATE schedules SET created_by = 'system' WHERE id = '{}'",
            g.schedule
        ),
        format!(
            "UPDATE schedules SET last_status = 'running' WHERE id = '{}'",
            g.schedule
        ),
        format!(
            "UPDATE schedules SET consecutive_failures = -1 WHERE id = '{}'",
            g.schedule
        ),
        format!(
            "UPDATE schedules SET enabled = false WHERE id = '{}'",
            g.schedule
        ),
        format!("UPDATE chats SET reasoning = 'max' WHERE id = '{}'", g.chat),
        format!(
            "UPDATE chats SET builtin_tools = '{{web}}' WHERE id = '{}'",
            g.chat
        ),
        format!(
            "UPDATE messages SET role = 'system' WHERE id = '{}'",
            g.reply
        ),
        format!(
            "UPDATE messages SET status = 'done' WHERE id = '{}'",
            g.reply
        ),
        format!(
            "UPDATE messages SET finish_reason = 'max_tokens' WHERE id = '{}'",
            g.reply
        ),
        format!(
            "UPDATE messages SET context_hash = decode(repeat('07', 31), 'hex') WHERE id = '{}'",
            g.reply
        ),
        format!(
            "UPDATE messages SET context_hash = decode(repeat('07', 32), 'hex') WHERE {user_message}"
        ),
        format!(
            "UPDATE turn_contexts SET reasoning = 'max' WHERE chat_id = '{}'",
            g.chat
        ),
        format!(
            "UPDATE turn_contexts SET hash = decode(repeat('07', 31), 'hex') WHERE chat_id = '{}'",
            g.chat
        ),
        format!(
            "UPDATE files SET chat_id = '{}' WHERE id = '{}'",
            g.chat, g.project_file
        ),
        format!(
            "UPDATE files SET project_id = NULL WHERE id = '{}'",
            g.project_file
        ),
        format!(
            "UPDATE files SET size_bytes = -1 WHERE id = '{}'",
            g.project_file
        ),
        format!(
            "UPDATE files SET token_estimate = -1 WHERE id = '{}'",
            g.project_file
        ),
        format!(
            "UPDATE files SET page_count = -1 WHERE id = '{}'",
            g.project_file
        ),
        format!(
            "UPDATE files SET sha256 = decode(repeat('09', 31), 'hex') WHERE id = '{}'",
            g.project_file
        ),
    ];
    for statement in &rejected {
        assert_eq!(
            fixtures::failure(&pool, statement).await.as_deref(),
            Some(CHECK_VIOLATION),
            "{statement}"
        );
    }

    let accepted = [
        format!(
            "UPDATE providers SET api = 'chat_completions' WHERE id = '{}'",
            g.provider
        ),
        format!(
            "UPDATE providers SET kind = 'anthropic', preset = 'custom', api = NULL WHERE id = '{}'",
            g.provider
        ),
        format!(
            "UPDATE tool_servers SET slug = 'abcdefghij12' WHERE id = '{}'",
            g.tool_server
        ),
        format!(
            "UPDATE tool_servers SET tools_cached = '{{}}', tools_fetched_at = now() WHERE id = '{}'",
            g.tool_server
        ),
        format!(
            "UPDATE projects SET builtin_tools = '{{schedules}}' WHERE id = '{}'",
            g.project
        ),
        format!(
            "UPDATE chats SET builtin_tools = '{{}}' WHERE id = '{}'",
            g.chat
        ),
        format!(
            "UPDATE schedules SET enabled = false, next_run_at = NULL WHERE id = '{}'",
            g.schedule
        ),
        format!(
            "UPDATE schedules SET enabled = true WHERE id = '{}'",
            g.schedule
        ),
        format!(
            "UPDATE files SET page_count = 3 WHERE id = '{}'",
            g.project_file
        ),
    ];
    for statement in &accepted {
        assert_eq!(
            fixtures::failure(&pool, statement).await,
            None,
            "{statement}"
        );
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn columns_default_as_specified(pool: DbPool) {
    fixtures::migrated(&pool).await;
    let user = fixtures::user(&pool).await;
    let g = Graph::insert(&pool, user).await;

    let checks = [
        ("providers", "enabled", g.provider, "true"),
        ("models", "enabled", g.model, "false"),
        ("models", "available", g.model, "true"),
        ("models", "capability_overrides::text", g.model, "{}"),
        ("tool_servers", "enabled", g.tool_server, "true"),
        (
            "tool_servers",
            "tools_review_required",
            g.tool_server,
            "false",
        ),
        ("tool_servers", "approval", g.tool_server, "ask"),
        ("tool_servers", "tool_modes::text", g.tool_server, "{}"),
        ("projects", "pinned", g.project, "false"),
        ("projects", "icon", g.project, ""),
        ("projects", "instructions", g.project, ""),
        ("projects", "builtin_tools::text", g.project, "{}"),
        ("projects", "coalesce(reasoning, 'null')", g.project, "null"),
        ("schedules", "enabled", g.schedule, "true"),
        ("schedules", "consecutive_failures", g.schedule, "0"),
        ("schedules", "instructions", g.schedule, ""),
        ("schedules", "on_ask", g.schedule, "deny"),
        ("chats", "pinned", g.chat, "false"),
        ("chats", "voice_replies", g.chat, "false"),
        ("chats", "instructions", g.chat, ""),
        (
            "chats",
            "coalesce(builtin_tools::text, 'null')",
            g.chat,
            "null",
        ),
        ("files", "included", g.project_file, "true"),
    ];
    for (table, expression, id, expected) in checks {
        assert_eq!(
            value(&pool, table, expression, id).await,
            expected,
            "{table}.{expression}"
        );
    }

    let seq: Vec<i64> =
        sqlx::query_scalar("SELECT seq FROM messages WHERE chat_id = $1 ORDER BY seq")
            .bind(g.chat)
            .fetch_all(&pool)
            .await
            .expect("messages read");
    assert_eq!(seq.len(), 2);
    assert!(seq[0] < seq[1], "seq orders messages as written");
    let manual = format!(
        "INSERT INTO messages (seq, created_at, updated_at, id, user_id, chat_id, role, status, parts) \
         VALUES (1, now(), now(), '{}', '{}', '{}', 'user', 'complete', '[]')",
        Uuid::now_v7(),
        g.user,
        g.chat
    );
    assert!(
        fixtures::failure(&pool, &manual).await.is_some(),
        "seq is generated always"
    );
}

/// `expression` of the row of `table` with `id`, as text.
async fn value(pool: &DbPool, table: &str, expression: &str, id: Uuid) -> String {
    let sql = format!("SELECT ({expression})::text FROM {table} WHERE id = $1");
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("value read")
}
