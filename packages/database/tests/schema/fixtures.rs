//! Rows for the schema tests: users in lib-rustauth's `auth.users`, and one of every Copilot row
//! for a user.
//!
//! Copilot code never writes to the `auth` schema; these fixtures do, because the tests need
//! accounts without lib-rustauth's services. They write only the columns `auth.users` requires.

use copilot_database::DbPool;
use copilot_database::operations::migrate;
use sqlx::Postgres;
use sqlx::postgres::PgArguments;
use sqlx::query::Query;
use uuid::Uuid;

/// The SQLSTATE of a `CHECK` violation.
pub const CHECK_VIOLATION: &str = "23514";
/// The SQLSTATE of a foreign key violation.
pub const FOREIGN_KEY_VIOLATION: &str = "23503";
/// The SQLSTATE of a missing privilege.
pub const INSUFFICIENT_PRIVILEGE: &str = "42501";

/// Every table Copilot's migrations create in `public`, sorted (the spec's Data Model).
pub const TABLES: &[&str] = &[
    "chat_tool_approvals",
    "chat_tool_servers",
    "chats",
    "file_contents",
    "files",
    "messages",
    "models",
    "project_tool_servers",
    "projects",
    "providers",
    "schedules",
    "tool_servers",
    "turn_contexts",
    "user_settings",
];

/// Applies lib-rustauth's migrations, then Copilot's, like `make migrate`.
pub async fn migrated(pool: &DbPool) {
    migrate::run(pool).await.expect("migrations apply");
}

/// A new account in `auth.users`.
pub async fn user(pool: &DbPool) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO auth.users (created_at, updated_at, id, username, display_name, role) \
         VALUES (now(), now(), $1, $2, $2, 'user')",
    )
    .bind(id)
    .bind(id.to_string())
    .execute(pool)
    .await
    .expect("user inserted");
    id
}

/// The tables in `schema`, sorted.
pub async fn tables(pool: &DbPool, schema: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables \
         WHERE table_schema = $1 AND table_type = 'BASE TABLE' ORDER BY table_name",
    )
    .bind(schema)
    .fetch_all(pool)
    .await
    .expect("tables listed")
}

/// One of every Copilot row for one user, with the ids the tests look up.
#[derive(Clone, Copy, Debug)]
pub struct Graph {
    pub user: Uuid,
    pub provider: Uuid,
    pub model: Uuid,
    pub tool_server: Uuid,
    pub project: Uuid,
    /// In the project, on the model.
    pub schedule: Uuid,
    /// In the project, on the model, started by the schedule.
    pub chat: Uuid,
    /// Standalone, on the model; the schedule's source chat.
    pub standalone: Uuid,
    /// An assistant message of `chat`, from the provider and model.
    pub reply: Uuid,
    pub project_file: Uuid,
    pub chat_file: Uuid,
}

impl Graph {
    /// Inserts a provider with a model, a tool server, a project that uses both, a schedule in
    /// the project, two chats (one started by the schedule, one standalone that created it) with
    /// tool overrides, an approval, messages, and a turn snapshot, a project file and a chat
    /// file with their bytes, and the user's settings.
    pub async fn insert(pool: &DbPool, user: Uuid) -> Self {
        let graph = Self {
            user,
            provider: Uuid::now_v7(),
            model: Uuid::now_v7(),
            tool_server: Uuid::now_v7(),
            project: Uuid::now_v7(),
            schedule: Uuid::now_v7(),
            chat: Uuid::now_v7(),
            standalone: Uuid::now_v7(),
            reply: Uuid::now_v7(),
            project_file: Uuid::now_v7(),
            chat_file: Uuid::now_v7(),
        };
        let statements = [
            sqlx::query(
                "INSERT INTO user_settings (created_at, updated_at, user_id, instructions) \
                 VALUES (now(), now(), $1, 'Answer briefly.')",
            )
            .bind(user),
            sqlx::query(
                "INSERT INTO providers \
                 (created_at, updated_at, id, user_id, kind, preset, name, base_url, api_key_sealed) \
                 VALUES (now(), now(), $1, $2, 'openai', 'openai', 'OpenAI', \
                 'https://api.openai.com/v1', '\\x01')",
            )
            .bind(graph.provider)
            .bind(user),
            sqlx::query(
                "INSERT INTO models \
                 (created_at, updated_at, id, user_id, provider_id, remote_id, display_name, capabilities) \
                 VALUES (now(), now(), $1, $2, $3, 'gpt-5', 'GPT-5', '{}')",
            )
            .bind(graph.model)
            .bind(user)
            .bind(graph.provider),
            sqlx::query(
                "INSERT INTO tool_servers \
                 (created_at, updated_at, id, user_id, name, slug, url, headers_sealed) \
                 VALUES (now(), now(), $1, $2, 'Docs', 'docs', 'https://mcp.example.com', '\\x02')",
            )
            .bind(graph.tool_server)
            .bind(user),
            sqlx::query(
                "INSERT INTO projects (created_at, updated_at, id, user_id, default_model_id, name) \
                 VALUES (now(), now(), $1, $2, $3, 'Research')",
            )
            .bind(graph.project)
            .bind(user)
            .bind(graph.model),
            sqlx::query(
                "INSERT INTO project_tool_servers (project_id, tool_server_id, user_id) \
                 VALUES ($1, $2, $3)",
            )
            .bind(graph.project)
            .bind(graph.tool_server)
            .bind(user),
            sqlx::query(
                "INSERT INTO schedules \
                 (created_at, updated_at, next_run_at, id, user_id, project_id, model_id, name, \
                  cron, timezone, prompt, created_by) \
                 VALUES (now(), now(), now(), $1, $2, $3, $4, 'Daily', '0 9 * * *', 'UTC', \
                  'Summarize the news.', 'user')",
            )
            .bind(graph.schedule)
            .bind(user)
            .bind(graph.project)
            .bind(graph.model),
            chat(graph.chat, user, graph.model)
                .bind(Some(graph.project))
                .bind(Some(graph.schedule)),
            chat(graph.standalone, user, graph.model)
                .bind(None::<Uuid>)
                .bind(None::<Uuid>),
            sqlx::query("UPDATE schedules SET source_chat_id = $1 WHERE id = $2")
                .bind(graph.standalone)
                .bind(graph.schedule),
            sqlx::query(
                "INSERT INTO chat_tool_servers (chat_id, tool_server_id, user_id, enabled) \
                 VALUES ($1, $2, $3, true)",
            )
            .bind(graph.standalone)
            .bind(graph.tool_server)
            .bind(user),
            sqlx::query(
                "INSERT INTO chat_tool_approvals (chat_id, tool_server_id, user_id, tool_name) \
                 VALUES ($1, $2, $3, 'search')",
            )
            .bind(graph.chat)
            .bind(graph.tool_server)
            .bind(user),
            sqlx::query(
                "INSERT INTO messages \
                 (created_at, updated_at, id, user_id, chat_id, role, status, parts, text_content) \
                 VALUES (now(), now(), $1, $2, $3, 'user', 'complete', '[]', 'Hello')",
            )
            .bind(Uuid::now_v7())
            .bind(user)
            .bind(graph.chat),
            sqlx::query(
                "INSERT INTO messages \
                 (created_at, updated_at, id, user_id, chat_id, model_id, provider_id, role, \
                  status, finish_reason, context_hash, parts, text_content) \
                 VALUES (now(), now(), $1, $2, $3, $4, $5, 'assistant', 'complete', 'stop', \
                  $6, '[]', 'Hi')",
            )
            .bind(graph.reply)
            .bind(user)
            .bind(graph.chat)
            .bind(graph.model)
            .bind(graph.provider)
            .bind(vec![7_u8; 32]),
            sqlx::query(
                "INSERT INTO turn_contexts (created_at, chat_id, user_id, hash, system, tools, files) \
                 VALUES (now(), $1, $2, $3, 'You are helpful.', '[]', '[]')",
            )
            .bind(graph.chat)
            .bind(user)
            .bind(vec![7_u8; 32]),
            file(graph.project_file, user)
                .bind(Some(graph.project))
                .bind(None::<Uuid>),
            file(graph.chat_file, user)
                .bind(None::<Uuid>)
                .bind(Some(graph.chat)),
            content(graph.project_file),
            content(graph.chat_file),
        ];
        for statement in statements {
            statement.execute(pool).await.expect("fixture row inserted");
        }
        graph
    }
}

/// A chat insert on `model`; bind `project_id` and `schedule_id` next.
fn chat(id: Uuid, user: Uuid, model: Uuid) -> Query<'static, Postgres, PgArguments> {
    sqlx::query(
        "INSERT INTO chats \
         (created_at, updated_at, id, user_id, model_id, title, project_id, schedule_id) \
         VALUES (now(), now(), $1, $2, $3, 'Chat', $4, $5)",
    )
    .bind(id)
    .bind(user)
    .bind(model)
}

/// A file insert; bind `project_id` and `chat_id` next.
fn file(id: Uuid, user: Uuid) -> Query<'static, Postgres, PgArguments> {
    sqlx::query(
        "INSERT INTO files \
         (created_at, size_bytes, token_estimate, id, user_id, name, media_type, sha256, \
          project_id, chat_id) \
         VALUES (now(), 5, 2, $1, $2, 'notes.txt', 'text/plain', $3, $4, $5)",
    )
    .bind(id)
    .bind(user)
    .bind(vec![9_u8; 32])
}

fn content(file: Uuid) -> Query<'static, Postgres, PgArguments> {
    sqlx::query("INSERT INTO file_contents (file_id, bytes) VALUES ($1, 'hello')").bind(file)
}

/// How many rows of `table` match `filter` (a SQL condition on `$1`).
pub async fn count(pool: &DbPool, table: &str, filter: &str, id: Uuid) -> i64 {
    let sql = format!("SELECT count(*) FROM {table} WHERE {filter}");
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("rows counted")
}

/// The SQLSTATE `sql` fails with, or `None` when it succeeds.
pub async fn failure<'c, E>(executor: E, sql: &str) -> Option<String>
where
    E: sqlx::Executor<'c, Database = Postgres>,
{
    match sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned()))
        .execute(executor)
        .await
    {
        Ok(_) => None,
        Err(sqlx::Error::Database(error)) => error.code().map(|code| code.into_owned()),
        Err(error) => panic!("unexpected error for {sql}: {error}"),
    }
}
