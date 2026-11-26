//! Repository tests (`make test-db`): every row and stored JSON record round-trips, every read
//! and write is scoped to its owner, multi-row writes are one transaction, a refresh syncs
//! models, and message writes move the chat in the recent list.
//!
//! Every test gets a throwaway database (`#[sqlx::test(migrations = false)]`) and starts with
//! [`store`], which runs the same `migrate::run` as a real install. The store's clock stands still
//! at [`NOW`], so the timestamps a repository sets itself are known.

mod chats;
mod files;
mod projects;
mod providers;
mod schedules;
mod settings;
mod tools;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use copilot_core::{
    BaseUrl, Capabilities, ChatRepository, Clock, DetectedModel, Model, ModelQuery,
    ModelRepository, NetworkScope, Preset, Provider, ProviderKind, ProviderRepository,
    ReasoningLevel, RemoteModel, ReportedCapabilities, Slug, ToolServer, ToolServerRepository,
    UserId,
};
use copilot_database::operations::migrate;
use copilot_database::{DbPool, PgStore};
use uuid::Uuid;

/// The models a connection keeps, as `ProviderService::refresh` passes it.
pub const KEEP: usize = copilot_core::shared::constants::MAX_MODELS_PER_PROVIDER;

/// The store clock's time, in seconds after [`at`]'s instant.
pub const NOW: i64 = 86_400;

/// A clock that always reads `at(NOW)`.
struct Still;

impl Clock for Still {
    fn now(&self) -> DateTime<Utc> {
        at(NOW)
    }
}

/// A migrated store whose clock reads `at(NOW)`.
pub async fn store(pool: DbPool) -> PgStore {
    migrate::run(&pool).await.expect("migrations apply");
    PgStore::new(pool, Arc::new(Still))
}

/// A new account in lib-rustauth's `auth.users` (Copilot code never writes there; the tests need
/// accounts without lib-rustauth's services).
pub async fn user(store: &PgStore) -> UserId {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO auth.users (created_at, updated_at, id, username, display_name, role) \
         VALUES (now(), now(), $1, $2, $2, 'user')",
    )
    .bind(id)
    .bind(id.to_string())
    .execute(store.pool())
    .await
    .expect("user inserted");
    UserId::new(id)
}

/// A time Postgres stores exactly (microseconds), `seconds` after a fixed instant.
pub fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000 + seconds, 123_456_000).expect("a valid instant")
}

/// A saved OpenAI-compatible connection of `user`.
pub async fn provider(store: &PgStore, user: UserId) -> Provider {
    let url = BaseUrl::parse("https://api.example.com/v1", NetworkScope::Public).unwrap();
    let provider = Provider::create(
        user,
        ProviderKind::OpenAi,
        Preset::Custom,
        "Gateway",
        url,
        at(0),
    )
    .unwrap()
    .with_key(Some(vec![1, 2, 3]));
    ProviderRepository::insert(store, &provider).await.unwrap();
    provider
}

/// A saved tool server of `owner` with the slug `slug`.
pub async fn server(store: &PgStore, owner: UserId, slug: &str) -> ToolServer {
    let url = BaseUrl::parse("https://mcp.example.com", NetworkScope::Public).unwrap();
    let server = ToolServer::create(owner, "Docs", Slug::parse(slug).unwrap(), url, at(0)).unwrap();
    ToolServerRepository::insert(store, &server).await.unwrap();
    server
}

/// A listed model with reasoning levels.
pub fn detected(remote_id: &str, context_window: Option<u32>) -> DetectedModel {
    DetectedModel {
        remote: RemoteModel {
            remote_id: remote_id.to_owned(),
            display_name: remote_id.to_uppercase(),
            context_window,
            max_output_tokens: None,
            reported: ReportedCapabilities::default(),
        },
        capabilities: Capabilities {
            tools: true,
            reasoning: true,
            reasoning_levels: vec![ReasoningLevel::Low, ReasoningLevel::High],
            ..Capabilities::default()
        },
    }
}

/// An enabled model of a new connection of `user`.
pub async fn model(store: &PgStore, user: UserId) -> Model {
    let provider = provider(store, user).await;
    store
        .sync(
            user,
            provider.id(),
            vec![detected("gpt", Some(128_000))],
            true,
            KEEP,
        )
        .await
        .unwrap();
    ModelRepository::list(store, user, ModelQuery::provider(provider.id()))
        .await
        .unwrap()
        .remove(0)
}

/// A small text file of `owner` in a project or a chat, written with SQL (the file repository
/// has its own tests); its id.
pub async fn file_in(
    store: &PgStore,
    owner: UserId,
    project: Option<copilot_core::ProjectId>,
    chat: Option<copilot_core::ChatId>,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO files (created_at, size_bytes, token_estimate, id, user_id, project_id, \
         chat_id, name, media_type, sha256) \
         VALUES ($1, 5, 2, $2, $3, $4, $5, 'notes.txt', 'text/plain', $6)",
    )
    .bind(at(0))
    .bind(id)
    .bind(owner.uuid())
    .bind(project.map(copilot_core::ProjectId::uuid))
    .bind(chat.map(copilot_core::ChatId::uuid))
    .bind(vec![0_u8; 32])
    .execute(store.pool())
    .await
    .expect("file inserted");
    sqlx::query("INSERT INTO file_contents (file_id, bytes) VALUES ($1, $2)")
        .bind(id)
        .bind(b"hello".to_vec())
        .execute(store.pool())
        .await
        .expect("content inserted");
    id
}

/// The chat's `updated_at` as stored.
pub async fn updated_at(
    store: &PgStore,
    user: UserId,
    chat: copilot_core::ChatId,
) -> DateTime<Utc> {
    ChatRepository::get(store, user, chat)
        .await
        .unwrap()
        .expect("the chat exists")
        .updated_at()
}
