//! Tool server persistence port.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domains::tool::entities::ToolServer;
use crate::ports::tool_host::ToolListing;
use crate::shared::base_url::BaseUrl;
use crate::shared::errors::DomainResult;
use crate::shared::ids::{ToolServerId, UserId};

#[async_trait]
pub trait ToolServerRepository: Send + Sync {
    /// The user's servers, oldest first.
    async fn list(&self, user: UserId) -> DomainResult<Vec<ToolServer>>;
    async fn get(&self, user: UserId, id: ToolServerId) -> DomainResult<Option<ToolServer>>;
    /// `UNIQUE (user_id, slug)`: the service picks a free slug first.
    async fn insert(&self, server: &ToolServer) -> DomainResult<()>;
    /// Writes the user's settings: name, URL, credentials, approval, tool modes, enabled,
    /// protocol version, and the review flag. The slug never changes.
    async fn update(&self, server: &ToolServer) -> DomainResult<()>;
    /// Cascades to the `project_tool_servers` and `chat_tool_servers` rows. `false` when the
    /// user has no such server.
    async fn delete(&self, user: UserId, id: ToolServerId) -> DomainResult<bool>;
    /// Stores a `tools/list` result of `url`: `tools_cached` (with `ttlMs`),
    /// `tools_fetched_at = at`, `protocol_version`, and `tools_review_required`. Stores nothing
    /// and returns `false` when the server's URL is no longer `url` (the user changed it while
    /// the list was fetched, so the list belongs to the old URL) or the user has no such server.
    async fn save_listing(
        &self,
        user: UserId,
        id: ToolServerId,
        url: &BaseUrl,
        listing: &ToolListing,
        review_required: bool,
        at: DateTime<Utc>,
    ) -> DomainResult<bool>;
}
