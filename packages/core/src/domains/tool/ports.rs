//! Tool server persistence ports.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domains::tool::auth::{SignIn, SignInFlow};
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
    /// Writes the user's settings: name, URL, credentials (headers, auth mode, the OAuth
    /// client), approval, tool modes, enabled, protocol version, and the review flag. The slug
    /// never changes. It never stores a sign-in ([`ToolServerRepository::save_sign_in`] does): it
    /// clears the stored one when the new auth mode is not `oauth` or the URL changes, and keeps
    /// it otherwise, so a settings save never undoes a sign-in or a refresh that ran meanwhile.
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
    /// Stores `sign_in`, or clears it with `None`, when the server still has `url` and the auth
    /// mode `oauth`, and, with `previous`, still holds exactly that sealed sign-in (so two
    /// refreshes never both write). `false`, with nothing stored, otherwise.
    async fn save_sign_in(
        &self,
        user: UserId,
        id: ToolServerId,
        url: &BaseUrl,
        sign_in: Option<&SignIn>,
        previous: Option<&[u8]>,
    ) -> DomainResult<bool>;
    /// Marks the stored tool list expired (its `ttlMs` becomes 0; `tools_fetched_at` stays), so
    /// the next turn lists the server again and keeps the old list only if that fails. Nothing
    /// happens when the server has no list or the user has no such server.
    async fn stale(&self, user: UserId, id: ToolServerId) -> DomainResult<()>;
}

/// Sign-ins in progress (`oauth_flows`).
#[async_trait]
pub trait SignInRepository: Send + Sync {
    /// Stores `flow`, replacing any other flow of the same server, and deletes expired flows.
    async fn insert(&self, flow: &SignInFlow) -> DomainResult<()>;
    /// Removes and returns the flow whose `state_hash` matches, so a `state` works at most once.
    /// Expired flows are returned too; the caller checks `expires_at`.
    async fn take(&self, state_hash: &[u8]) -> DomainResult<Option<SignInFlow>>;
}
