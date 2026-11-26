//! Tool servers: registered remote MCP servers and their cached tool lists.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::domains::tool::values::{ApprovalMode, Slug, ToolMode};
use crate::ports::secret_cipher::SealContext;
use crate::ports::tool_host::{RemoteTool, ToolEndpoint, ToolListing};
use crate::shared::base_url::BaseUrl;
use crate::shared::constants::{NAME_MAX_CHARS, TOOL_CACHE_DEFAULT_TTL_SECS};
use crate::shared::errors::DomainResult;
use crate::shared::ids::{ToolServerId, UserId};
use crate::shared::net::NetworkScope;
use crate::shared::secret::Secret;
use crate::shared::text;

/// A user's MCP server (`tool_servers`). Credentials are sealed and bound to this row
/// ([`ToolServer::headers_context`]).
#[derive(Clone, Debug, PartialEq)]
pub struct ToolServer {
    id: ToolServerId,
    user_id: UserId,
    name: String,
    slug: Slug,
    url: BaseUrl,
    headers_sealed: Option<Vec<u8>>,
    approval: ApprovalMode,
    tool_modes: BTreeMap<String, ToolMode>,
    enabled: bool,
    protocol_version: Option<String>,
    cache: Option<ToolCache>,
    review_required: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Every field of a stored tool server, for [`ToolServer::restore`].
#[derive(Clone, Debug)]
pub struct ToolServerParts {
    pub id: ToolServerId,
    pub user_id: UserId,
    pub name: String,
    pub slug: Slug,
    pub url: BaseUrl,
    pub headers_sealed: Option<Vec<u8>>,
    pub approval: ApprovalMode,
    pub tool_modes: BTreeMap<String, ToolMode>,
    pub enabled: bool,
    pub protocol_version: Option<String>,
    /// `tools_cached` and `tools_fetched_at`.
    pub cache: Option<ToolCache>,
    /// `tools_review_required`.
    pub review_required: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The last `tools/list` result of a server, so building a request does not connect to every
/// server. Stored in `tools_cached` (with `ttlMs`) and `tools_fetched_at`.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCache {
    pub tools: Vec<RemoteTool>,
    pub ttl: Option<Duration>,
    pub fetched_at: DateTime<Utc>,
}

impl ToolCache {
    /// Past `fetched_at` plus the server's `ttlMs`, or `TOOL_CACHE_DEFAULT_TTL_SECS` without one.
    pub fn expired(&self, now: DateTime<Utc>) -> bool {
        let ttl = self
            .ttl
            .unwrap_or(Duration::from_secs(TOOL_CACHE_DEFAULT_TTL_SECS));
        // A TTL past the end of the calendar never expires.
        let ttl = chrono::Duration::from_std(ttl).unwrap_or(chrono::Duration::MAX);
        self.fetched_at
            .checked_add_signed(ttl)
            .is_some_and(|expiry| expiry <= now)
    }

    pub fn tool(&self, name: &str) -> Option<&RemoteTool> {
        self.tools.iter().find(|tool| tool.name == name)
    }
}

impl ToolServer {
    /// A new, enabled server without credentials or a tool list. Default approval: `ask`.
    pub fn create(
        user_id: UserId,
        name: &str,
        slug: Slug,
        url: BaseUrl,
        now: DateTime<Utc>,
    ) -> DomainResult<Self> {
        Ok(Self {
            id: ToolServerId::generate(),
            user_id,
            name: server_name(name)?,
            slug,
            url,
            headers_sealed: None,
            approval: ApprovalMode::Ask,
            tool_modes: BTreeMap::new(),
            enabled: true,
            protocol_version: None,
            cache: None,
            review_required: false,
            created_at: now,
            updated_at: now,
        })
    }

    /// A row read back from storage.
    pub fn restore(parts: ToolServerParts) -> Self {
        Self {
            id: parts.id,
            user_id: parts.user_id,
            name: parts.name,
            slug: parts.slug,
            url: parts.url,
            headers_sealed: parts.headers_sealed,
            approval: parts.approval,
            tool_modes: parts.tool_modes,
            enabled: parts.enabled,
            protocol_version: parts.protocol_version,
            cache: parts.cache,
            review_required: parts.review_required,
            created_at: parts.created_at,
            updated_at: parts.updated_at,
        }
    }

    pub fn with_name(mut self, name: &str) -> DomainResult<Self> {
        self.name = server_name(name)?;
        Ok(self)
    }

    /// A new URL forgets the negotiated protocol version (it may be another server). The cached
    /// tool list stays, so the next test compares against it and flags any change for review.
    #[must_use]
    pub fn with_url(mut self, url: BaseUrl) -> Self {
        if url != self.url {
            self.protocol_version = None;
        }
        self.url = url;
        self
    }

    /// Sets or clears the sealed credentials. Seal them with [`ToolServer::headers_context`].
    #[must_use]
    pub fn with_headers(mut self, sealed: Option<Vec<u8>>) -> Self {
        self.headers_sealed = sealed;
        self
    }

    #[must_use]
    pub fn with_approval(mut self, approval: ApprovalMode) -> Self {
        self.approval = approval;
        self
    }

    /// Replaces the per-tool modes. Check the names against the tool list first.
    #[must_use]
    pub fn with_tool_modes(mut self, modes: BTreeMap<String, ToolMode>) -> Self {
        self.tool_modes = modes;
        self
    }

    #[must_use]
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// What `ToolServerRepository::save_listing` stores.
    #[must_use]
    pub fn with_listing(
        mut self,
        listing: &ToolListing,
        review_required: bool,
        at: DateTime<Utc>,
    ) -> Self {
        self.cache = Some(ToolCache {
            tools: listing.tools.clone(),
            ttl: listing.ttl,
            fetched_at: at,
        });
        self.protocol_version = Some(listing.protocol_version.clone());
        self.review_required = review_required;
        self
    }

    /// The user accepted a changed tool list.
    #[must_use]
    pub fn reviewed(mut self) -> Self {
        self.review_required = false;
        self
    }

    #[must_use]
    pub fn touched(mut self, now: DateTime<Utc>) -> Self {
        self.updated_at = now;
        self
    }

    /// The associated data that binds the sealed credentials to this row.
    pub fn headers_context(&self) -> SealContext {
        SealContext {
            table: "tool_servers",
            column: "headers_sealed",
            row_id: self.id.uuid(),
            user_id: self.user_id,
        }
    }

    /// The mode of tool `name`: its own mode, else the server's approval mode.
    pub fn mode(&self, name: &str) -> ToolMode {
        self.tool_modes
            .get(name)
            .copied()
            .unwrap_or_else(|| self.approval.into())
    }

    /// The mode the tool list suggests for a tool without its own mode: `auto` for tools
    /// annotated `readOnlyHint` (a hint only), else the server's approval mode.
    pub fn suggested_mode(&self, tool: &RemoteTool) -> ToolMode {
        if tool.read_only {
            ToolMode::Auto
        } else {
            self.approval.into()
        }
    }

    /// The adapter's view of this server, with the opened credentials.
    pub fn endpoint(&self, headers: Option<Secret>, network: NetworkScope) -> ToolEndpoint {
        ToolEndpoint {
            server_id: self.id,
            url: self.url.clone(),
            headers,
            protocol_version: self.protocol_version.clone(),
            network,
        }
    }

    /// No tool list yet, or the cached one expired.
    pub fn needs_refresh(&self, now: DateTime<Utc>) -> bool {
        self.cache.as_ref().is_none_or(|cache| cache.expired(now))
    }

    pub fn id(&self) -> ToolServerId {
        self.id
    }

    pub fn user_id(&self) -> UserId {
        self.user_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn slug(&self) -> &Slug {
        &self.slug
    }

    pub fn url(&self) -> &BaseUrl {
        &self.url
    }

    pub fn headers_sealed(&self) -> Option<&[u8]> {
        self.headers_sealed.as_deref()
    }

    pub fn approval(&self) -> ApprovalMode {
        self.approval
    }

    pub fn tool_modes(&self) -> &BTreeMap<String, ToolMode> {
        &self.tool_modes
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn protocol_version(&self) -> Option<&str> {
        self.protocol_version.as_deref()
    }

    pub fn cache(&self) -> Option<&ToolCache> {
        self.cache.as_ref()
    }

    /// A refresh changed the tool list and the user has not reviewed it: every tool asks.
    pub fn review_required(&self) -> bool {
        self.review_required
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

fn server_name(name: &str) -> DomainResult<String> {
    text::name(name, "Tool server name", "name", NAME_MAX_CHARS)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    fn server() -> ToolServer {
        let url = BaseUrl::parse("https://mcp.example.com/mcp", NetworkScope::Public).unwrap();
        ToolServer::create(
            UserId::new(Uuid::now_v7()),
            "GitHub",
            Slug::parse("github").unwrap(),
            url,
            now(),
        )
        .unwrap()
    }

    fn tool(name: &str, read_only: bool) -> RemoteTool {
        RemoteTool {
            name: name.to_owned(),
            title: None,
            description: String::new(),
            input_schema: json!({"type": "object"}),
            read_only,
        }
    }

    fn listing(ttl: Option<Duration>) -> ToolListing {
        ToolListing {
            tools: vec![tool("search", true)],
            protocol_version: "2026-07-28".to_owned(),
            ttl,
        }
    }

    #[test]
    fn starts_enabled_with_ask_and_no_tools() {
        let server = server();
        assert_eq!(server.approval(), ApprovalMode::Ask);
        assert!(server.enabled());
        assert!(server.cache().is_none());
        assert!(server.needs_refresh(now()));
        assert!(!server.review_required());
    }

    #[test]
    fn falls_back_to_the_server_mode() {
        let server =
            server().with_tool_modes(BTreeMap::from([("delete".to_owned(), ToolMode::Off)]));
        assert_eq!(server.mode("delete"), ToolMode::Off);
        assert_eq!(server.mode("search"), ToolMode::Ask);
        let server = server.with_approval(ApprovalMode::Auto);
        assert_eq!(server.mode("search"), ToolMode::Auto);
    }

    #[test]
    fn suggests_auto_for_read_only_tools_only() {
        let server = server();
        assert_eq!(server.suggested_mode(&tool("search", true)), ToolMode::Auto);
        assert_eq!(server.suggested_mode(&tool("delete", false)), ToolMode::Ask);
    }

    #[test]
    fn expires_the_cache_after_the_server_ttl_or_one_hour() {
        let server = server().with_listing(&listing(None), false, now());
        assert_eq!(server.protocol_version(), Some("2026-07-28"));
        assert!(!server.needs_refresh(now() + chrono::Duration::minutes(59)));
        assert!(server.needs_refresh(now() + chrono::Duration::hours(1)));

        let short = server.with_listing(&listing(Some(Duration::from_secs(60))), false, now());
        assert!(!short.needs_refresh(now() + chrono::Duration::seconds(59)));
        assert!(short.needs_refresh(now() + chrono::Duration::seconds(60)));

        let huge = short.with_listing(&listing(Some(Duration::MAX)), false, now());
        assert!(!huge.needs_refresh(now() + chrono::Duration::days(365)));
    }

    #[test]
    fn a_new_url_forgets_the_protocol_version_but_keeps_the_tools() {
        let server = server().with_listing(&listing(None), false, now());
        let same = server.clone().with_url(server.url().clone());
        assert_eq!(same.protocol_version(), Some("2026-07-28"));
        let other = BaseUrl::parse("https://other.example.com/mcp", NetworkScope::Public).unwrap();
        let moved = server.with_url(other);
        assert_eq!(moved.protocol_version(), None);
        assert!(moved.cache().is_some());
    }

    #[test]
    fn review_flag_is_set_by_a_listing_and_cleared_by_a_review() {
        let server = server().with_listing(&listing(None), true, now());
        assert!(server.review_required());
        assert!(!server.reviewed().review_required());
    }
}
