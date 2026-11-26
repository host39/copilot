//! Provider and model persistence ports.

use async_trait::async_trait;

use crate::domains::provider::capabilities::Capabilities;
use crate::domains::provider::entities::{Model, Provider};
use crate::ports::llm_gateway::RemoteModel;
use crate::shared::errors::DomainResult;
use crate::shared::ids::{ModelId, ProviderId, UserId};

#[async_trait]
pub trait ProviderRepository: Send + Sync {
    /// The user's connections, oldest first.
    async fn list(&self, user: UserId) -> DomainResult<Vec<Provider>>;
    async fn get(&self, user: UserId, id: ProviderId) -> DomainResult<Option<Provider>>;
    async fn insert(&self, provider: &Provider) -> DomainResult<()>;
    async fn update(&self, provider: &Provider) -> DomainResult<()>;
    /// Deletes the connection and its models; references to those models become null. `false`
    /// when the user has no such connection.
    async fn delete(&self, user: UserId, id: ProviderId) -> DomainResult<bool>;
    /// Chats, projects, and schedules that would lose their model, for the delete confirmation.
    async fn usage(&self, user: UserId, id: ProviderId) -> DomainResult<ProviderUsage>;
}

#[async_trait]
pub trait ModelRepository: Send + Sync {
    /// Ordered by provider, then display name.
    async fn list(&self, user: UserId, query: ModelQuery) -> DomainResult<Vec<Model>>;
    async fn get(&self, user: UserId, id: ModelId) -> DomainResult<Option<Model>>;
    /// Applies a refresh of `provider`, in one transaction, keyed by `(provider_id, remote_id)`:
    ///
    /// - A listed model that is new is inserted with a v7 id, the remote display name and
    ///   limits, the detected capabilities, no overrides, and `enabled = auto_enable`.
    /// - A listed model that exists becomes available and gets the detected capabilities; a
    ///   reported context window or max output tokens replaces the stored value, an unreported
    ///   one keeps it. Its display name, `enabled`, and overrides are the user's and stay.
    /// - A stored model that is not listed becomes unavailable.
    /// - While the connection then holds more than `keep` models, models that are unavailable,
    ///   off, and named by no chat, message, project, or schedule are deleted, least recently
    ///   updated first. Nothing else is deleted.
    ///
    /// Rows it writes get `updated_at` (and `created_at` for inserts) from the repository's clock.
    async fn sync(
        &self,
        user: UserId,
        provider: ProviderId,
        remote: Vec<DetectedModel>,
        auto_enable: bool,
        keep: usize,
    ) -> DomainResult<SyncReport>;
    async fn update(&self, model: &Model) -> DomainResult<()>;
}

/// Counts for the delete confirmation of a provider.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProviderUsage {
    pub chats: u64,
    pub projects: u64,
    pub schedules: u64,
}

/// Which models to list.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ModelQuery {
    /// Only this connection's models.
    pub provider: Option<ProviderId>,
    /// Only models the picker offers: enabled, available, and on an enabled connection.
    pub usable: bool,
}

impl ModelQuery {
    /// The model picker.
    pub fn picker() -> Self {
        Self {
            provider: None,
            usable: true,
        }
    }

    /// Every known model of one connection.
    pub fn provider(id: ProviderId) -> Self {
        Self {
            provider: Some(id),
            usable: false,
        }
    }
}

/// A listed model with its detected capabilities (preset defaults and locks applied by core).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetectedModel {
    pub remote: RemoteModel,
    pub capabilities: Capabilities,
}

/// What a refresh changed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SyncReport {
    /// New models.
    pub added: u32,
    /// Models that were already known and are still listed.
    pub updated: u32,
    /// Known models the provider no longer lists.
    pub unavailable: u32,
    /// New models that were enabled automatically.
    pub enabled: u32,
}
