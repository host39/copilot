//! Tool servers: registered remote MCP servers and their cached tool lists.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::domains::tool::auth::{AuthMode, ClientConfig, SignIn, SignInState};
use crate::domains::tool::values::{ApprovalMode, Slug, ToolMode};
use crate::ports::secret_cipher::SealContext;
use crate::ports::tool_auth::TokenSource;
use crate::ports::tool_host::{RemoteTool, ToolEndpoint, ToolListing};
use crate::shared::base_url::BaseUrl;
use crate::shared::constants::{
    NAME_MAX_CHARS, TOOL_CACHE_DEFAULT_TTL_SECS, TOOL_CACHE_MAX_TTL_SECS,
};
use crate::shared::errors::DomainResult;
use crate::shared::ids::{ToolServerId, UserId};
use crate::shared::net::NetworkScope;
use crate::shared::secret::Secret;
use crate::shared::text;

/// A user's MCP server (`tool_servers`). Credentials are sealed and bound to this row
/// ([`ToolServer::headers_context`], [`ToolServer::client_context`],
/// [`ToolServer::sign_in_context`]).
///
/// The auth mode says which credentials requests carry. `Headers` holds exactly when headers are
/// stored; the OAuth client and sign-in exist only in `OAuth` mode.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolServer {
    id: ToolServerId,
    user_id: UserId,
    name: String,
    slug: Slug,
    url: BaseUrl,
    headers_sealed: Option<Vec<u8>>,
    auth: AuthMode,
    client: Option<ClientConfig>,
    sign_in: Option<SignIn>,
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
    /// `auth_mode`.
    pub auth: AuthMode,
    /// `oauth_client_id` and `oauth_client_secret_sealed`.
    pub client: Option<ClientConfig>,
    /// `oauth_tokens_sealed`, `oauth_expires_at`, and `oauth_refreshable`.
    pub sign_in: Option<SignIn>,
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
    /// Past `fetched_at` plus the server's `ttlMs` (at most `TOOL_CACHE_MAX_TTL_SECS`), or
    /// `TOOL_CACHE_DEFAULT_TTL_SECS` without one.
    pub fn expired(&self, now: DateTime<Utc>) -> bool {
        let ttl = self
            .ttl
            .unwrap_or(Duration::from_secs(TOOL_CACHE_DEFAULT_TTL_SECS))
            .min(Duration::from_secs(TOOL_CACHE_MAX_TTL_SECS));
        let ttl = chrono::Duration::from_std(ttl).unwrap_or(chrono::Duration::MAX);
        self.fetched_at
            .checked_add_signed(ttl)
            .is_none_or(|expiry| expiry <= now)
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
            auth: AuthMode::None,
            client: None,
            sign_in: None,
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
            auth: parts.auth,
            client: parts.client,
            sign_in: parts.sign_in,
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

    /// A new URL may be another server: it forgets the negotiated protocol version, so the
    /// cached tool list no longer counts as this URL's ([`ToolServer::listed`]) and the next turn
    /// lists the new URL first, and every tool asks until the user reviews the server again. The
    /// cached list stays only so the next listing can show what changed. It also ends the OAuth
    /// sign-in: its tokens were issued for the old URL (the `resource`).
    #[must_use]
    pub fn with_url(mut self, url: BaseUrl) -> Self {
        if url != self.url {
            self.protocol_version = None;
            self.review_required = true;
            self.sign_in = None;
        }
        self.url = url;
        self
    }

    /// Sets or clears the sealed bearer token and headers. Seal them with
    /// [`ToolServer::headers_context`]. Stored headers switch the server to `Headers` (ending any
    /// OAuth sign-in); clearing them leaves `Headers` for `None`.
    #[must_use]
    pub fn with_headers(mut self, sealed: Option<Vec<u8>>) -> Self {
        let stored = sealed.is_some();
        self.headers_sealed = sealed;
        if stored {
            self.with_auth(AuthMode::Headers)
        } else if self.auth == AuthMode::Headers {
            self.with_auth(AuthMode::None)
        } else {
            self
        }
    }

    /// Switches the auth mode, dropping the credentials of the other modes: headers outside
    /// `Headers`, the OAuth client and sign-in outside `OAuth`. Switching to `Headers` without
    /// headers leaves `None` (store them with [`ToolServer::with_headers`]).
    #[must_use]
    pub fn with_auth(mut self, mode: AuthMode) -> Self {
        if mode != AuthMode::Headers {
            self.headers_sealed = None;
        }
        if mode != AuthMode::OAuth {
            self.client = None;
            self.sign_in = None;
        }
        self.auth = if mode == AuthMode::Headers && self.headers_sealed.is_none() {
            AuthMode::None
        } else {
            mode
        };
        self
    }

    /// Sets or clears the pre-registered OAuth client (`OAuth` mode only; see
    /// [`ToolServer::with_auth`]). Seal its secret with [`ToolServer::client_context`].
    #[must_use]
    pub fn with_client(mut self, client: Option<ClientConfig>) -> Self {
        if self.auth == AuthMode::OAuth {
            self.client = client;
        }
        self
    }

    /// Sets or clears the sign-in (`OAuth` mode only). Seal it with
    /// [`ToolServer::sign_in_context`].
    #[must_use]
    pub fn with_sign_in(mut self, sign_in: Option<SignIn>) -> Self {
        if self.auth == AuthMode::OAuth {
            self.sign_in = sign_in;
        }
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

    /// The stored list may be out of date (a call named a tool the server no longer lists): it
    /// expires now, so the next turn lists the server again, and stays as the fallback.
    #[must_use]
    pub fn stale(mut self) -> Self {
        if let Some(cache) = &mut self.cache {
            cache.ttl = Some(Duration::ZERO);
        }
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

    /// The associated data of the pre-registered client's secret.
    pub fn client_context(&self) -> SealContext {
        SealContext {
            column: "oauth_client_secret_sealed",
            ..self.headers_context()
        }
    }

    /// The associated data of the sealed sign-in.
    pub fn sign_in_context(&self) -> SealContext {
        SealContext {
            column: "oauth_tokens_sealed",
            ..self.headers_context()
        }
    }

    /// What the settings show for an `OAuth` server; `None` in the other modes.
    pub fn sign_in_state(&self, now: DateTime<Utc>) -> Option<SignInState> {
        (self.auth == AuthMode::OAuth).then(|| {
            self.sign_in
                .as_ref()
                .map_or(SignInState::NeedsSignIn, |sign_in| sign_in.state(now))
        })
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

    /// The adapter's view of this server, with the opened headers or the sign-in's tokens.
    pub fn endpoint(
        &self,
        headers: Option<Secret>,
        tokens: Option<TokenSource>,
        network: NetworkScope,
    ) -> ToolEndpoint {
        ToolEndpoint {
            server_id: self.id,
            url: self.url.clone(),
            headers,
            tokens,
            protocol_version: self.protocol_version.clone(),
            network,
        }
    }

    /// No tool list from the current URL yet, or the cached one expired.
    pub fn needs_refresh(&self, now: DateTime<Utc>) -> bool {
        !self.listed() || self.cache.as_ref().is_none_or(|cache| cache.expired(now))
    }

    /// The cached tool list came from the current URL. A listing always stores the protocol
    /// version and a new URL clears it, so a cache without one belongs to an old URL.
    pub fn listed(&self) -> bool {
        self.cache.is_some() && self.protocol_version.is_some()
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

    pub fn auth(&self) -> AuthMode {
        self.auth
    }

    pub fn client(&self) -> Option<&ClientConfig> {
        self.client.as_ref()
    }

    pub fn sign_in(&self) -> Option<&SignIn> {
        self.sign_in.as_ref()
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

        let day = short.with_listing(&listing(Some(Duration::from_secs(172_800))), false, now());
        assert!(!day.needs_refresh(now() + chrono::Duration::hours(23)));
        assert!(
            day.needs_refresh(now() + chrono::Duration::hours(24)),
            "a server cannot keep its list for longer than a day"
        );
        let huge = day.with_listing(&listing(Some(Duration::MAX)), false, now());
        assert!(huge.needs_refresh(now() + chrono::Duration::days(365)));
    }

    #[test]
    fn a_new_url_needs_a_new_listing_and_a_review() {
        let server = server().with_listing(&listing(None), false, now());
        let same = server.clone().with_url(server.url().clone());
        assert_eq!(same.protocol_version(), Some("2026-07-28"));
        assert!(!same.review_required());
        assert!(!same.needs_refresh(now()));
        let other = BaseUrl::parse("https://other.example.com/mcp", NetworkScope::Public).unwrap();
        let moved = server.with_url(other);
        assert_eq!(moved.protocol_version(), None);
        assert!(moved.cache().is_some(), "kept to show what changed");
        assert!(!moved.listed());
        assert!(moved.needs_refresh(now()));
        assert!(moved.review_required(), "every tool asks until a review");
    }

    fn signed_in() -> ToolServer {
        server()
            .with_auth(AuthMode::OAuth)
            .with_client(Some(ClientConfig {
                id: "client-1".to_owned(),
                secret_sealed: None,
            }))
            .with_sign_in(Some(SignIn {
                sealed: vec![1, 2, 3],
                expires_at: None,
                refreshable: true,
            }))
    }

    #[test]
    fn headers_and_oauth_exclude_each_other() {
        let server = server();
        assert_eq!(server.auth(), AuthMode::None);
        assert_eq!(server.sign_in_state(now()), None);

        let headers = server.clone().with_headers(Some(vec![9]));
        assert_eq!(headers.auth(), AuthMode::Headers);
        let cleared = headers.clone().with_headers(None);
        assert_eq!(
            cleared.auth(),
            AuthMode::None,
            "no headers, no headers mode"
        );

        let oauth = signed_in();
        assert_eq!(oauth.auth(), AuthMode::OAuth);
        assert_eq!(oauth.headers_sealed(), None);
        assert_eq!(oauth.sign_in_state(now()), Some(SignInState::SignedIn));
        let switched = oauth.clone().with_headers(Some(vec![9]));
        assert_eq!(switched.auth(), AuthMode::Headers);
        assert!(switched.client().is_none() && switched.sign_in().is_none());

        let back = headers.with_auth(AuthMode::OAuth);
        assert_eq!(back.headers_sealed(), None, "OAuth drops the headers");
        assert_eq!(back.sign_in_state(now()), Some(SignInState::NeedsSignIn));
        assert_eq!(
            server.clone().with_auth(AuthMode::Headers).auth(),
            AuthMode::None,
            "headers mode needs headers"
        );
        let none = oauth.with_auth(AuthMode::None);
        assert!(none.client().is_none() && none.sign_in().is_none());
        assert!(
            server
                .with_sign_in(Some(SignIn {
                    sealed: vec![1],
                    expires_at: None,
                    refreshable: false,
                }))
                .sign_in()
                .is_none(),
            "only an OAuth server keeps a sign-in"
        );
    }

    #[test]
    fn a_new_url_ends_the_sign_in_but_keeps_the_client() {
        let server = signed_in();
        let same = server.clone().with_url(server.url().clone());
        assert!(same.sign_in().is_some());
        let other = BaseUrl::parse("https://other.example.com/mcp", NetworkScope::Public).unwrap();
        let moved = server.with_url(other);
        assert!(moved.sign_in().is_none());
        assert!(moved.client().is_some());
        assert_eq!(moved.sign_in_state(now()), Some(SignInState::NeedsSignIn));
    }

    #[test]
    fn binds_each_sealed_column_to_the_row() {
        let server = server();
        assert_eq!(server.headers_context().column, "headers_sealed");
        assert_eq!(server.client_context().column, "oauth_client_secret_sealed");
        assert_eq!(server.sign_in_context().column, "oauth_tokens_sealed");
        assert_eq!(server.sign_in_context().row_id, server.id().uuid());
        assert_eq!(server.sign_in_context().user_id, server.user_id());
    }

    #[test]
    fn review_flag_is_set_by_a_listing_and_cleared_by_a_review() {
        let server = server().with_listing(&listing(None), true, now());
        assert!(server.review_required());
        assert!(!server.reviewed().review_required());
    }
}
