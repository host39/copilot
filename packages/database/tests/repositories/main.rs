//! Tests of the repositories the walking skeleton uses (`make test-db`): every row and stored
//! JSON record round-trips, every read and write is scoped to its owner, a refresh syncs models,
//! and message writes move the chat in the recent list.
//!
//! Every test gets a throwaway database (`#[sqlx::test(migrations = false)]`) and starts with
//! [`store`], which runs the same `migrate::run` as a real install. The store's clock stands still
//! at [`NOW`], so the timestamps a repository sets itself are known.

mod chats;
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
    ReasoningLevel, RemoteModel, ReportedCapabilities, UserId,
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
