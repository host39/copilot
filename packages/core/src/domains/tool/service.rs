//! Tool server use cases: settings, tests (tool listing and change detection), reviews, and the
//! tools a turn gets.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::sync::Arc;

use crate::domains::provider::KeyState;
use crate::domains::tool::entities::ToolServer;
use crate::domains::tool::headers::ToolHeaders;
use crate::domains::tool::ports::ToolServerRepository;
use crate::domains::tool::selection::{EffectiveTools, ToolChanges, ToolSelection, offered};
use crate::domains::tool::values::{ApprovalMode, Slug, ToolMode};
use crate::ports::clock::Clock;
use crate::ports::secret_cipher::{SECRET_UNREADABLE, SecretCipher};
use crate::ports::tool_host::{ToolEndpoint, ToolHost, ToolListing};
use crate::shared::actor::Actor;
use crate::shared::base_url::BaseUrl;
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::ToolServerId;
use crate::shared::net::PrivateNetwork;
use crate::shared::secret::REDACTED;

/// Input of [`ToolService::create`]. `Debug` prints `***` for the token and header values.
#[derive(Clone, Default)]
pub struct NewToolServer {
    pub name: String,
    /// `None` derives a free slug from the name.
    pub slug: Option<String>,
    pub url: String,
    /// Blank means none.
    pub bearer: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub approval: ApprovalMode,
}

impl fmt::Debug for NewToolServer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NewToolServer")
            .field("name", &self.name)
            .field("slug", &self.slug)
            .field("url", &self.url)
            .field("bearer", &self.bearer.as_ref().map(|_| REDACTED))
            .field("headers", &Redacted(&self.headers))
            .field("approval", &self.approval)
            .finish()
    }
}

/// Input of [`ToolService::update`]; `None` keeps a field. The slug never changes.
#[derive(Clone, Debug, Default)]
pub struct ToolServerUpdate {
    pub name: Option<String>,
    pub url: Option<String>,
    pub credentials: Option<CredentialChange>,
    pub approval: Option<ApprovalMode>,
    /// Replaces every per-tool mode. Names must be on the server's tool list.
    pub tool_modes: Option<BTreeMap<String, ToolMode>>,
    pub enabled: Option<bool>,
}

/// A change to the stored bearer token and headers. `Debug` prints `***` for the token and
/// header values.
#[derive(Clone, Eq, PartialEq)]
pub enum CredentialChange {
    /// Replaces both; all blank clears them.
    Set {
        bearer: Option<String>,
        headers: BTreeMap<String, String>,
    },
    Clear,
}

impl fmt::Debug for CredentialChange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Set { bearer, headers } => formatter
                .debug_struct("Set")
                .field("bearer", &bearer.as_ref().map(|_| REDACTED))
                .field("headers", &Redacted(headers))
                .finish(),
            Self::Clear => formatter.write_str("Clear"),
        }
    }
}

/// Header names with `***` for each value.
struct Redacted<'a>(&'a BTreeMap<String, String>);

impl fmt::Debug for Redacted<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_map()
            .entries(self.0.keys().map(|name| (name, REDACTED)))
            .finish()
    }
}

/// A server with whether its credentials still open ("Key must be re-entered").
#[derive(Clone, Debug, PartialEq)]
pub struct ToolServerDetails {
    pub server: ToolServer,
    pub credentials: KeyState,
}

/// The result of a test: the server with its new tool list, and what changed since the last one.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolTest {
    pub server: ToolServer,
    pub changes: ToolChanges,
}

#[derive(Clone)]
pub struct ToolService {
    servers: Arc<dyn ToolServerRepository>,
    host: Arc<dyn ToolHost>,
    cipher: Arc<dyn SecretCipher>,
    clock: Arc<dyn Clock>,
    network: PrivateNetwork,
}

impl ToolService {
    /// `network` is `ALLOW_PRIVATE_NETWORK`.
    pub fn new(
        servers: Arc<dyn ToolServerRepository>,
        host: Arc<dyn ToolHost>,
        cipher: Arc<dyn SecretCipher>,
        clock: Arc<dyn Clock>,
        network: PrivateNetwork,
    ) -> Self {
        Self {
            servers,
            host,
            cipher,
            clock,
            network,
        }
    }

    pub async fn list(&self, actor: &Actor) -> DomainResult<Vec<ToolServer>> {
        self.servers.list(actor.user_id).await
    }

    pub async fn get(&self, actor: &Actor, id: ToolServerId) -> DomainResult<ToolServerDetails> {
        let server = self.load(actor, id).await?;
        let credentials = match server.headers_sealed() {
            None => KeyState::Missing,
            Some(sealed) => match self.cipher.open(sealed, &server.headers_context()) {
                Ok(_) => KeyState::Set,
                Err(error) if error.code() == SECRET_UNREADABLE => KeyState::Unreadable,
                Err(error) => return Err(error),
            },
        };
        Ok(ToolServerDetails {
            server,
            credentials,
        })
    }

    /// Registers a server. It has no tools until the first test.
    pub async fn create(&self, actor: &Actor, input: NewToolServer) -> DomainResult<ToolServer> {
        let url = self.url(actor, &input.url)?;
        let headers = ToolHeaders::new(input.bearer.as_deref(), &input.headers)?;
        let taken: Vec<Slug> = self
            .servers
            .list(actor.user_id)
            .await?
            .iter()
            .map(|server| server.slug().clone())
            .collect();
        let slug = match input.slug.as_deref() {
            Some(text) => {
                let slug = Slug::parse(text)?;
                if taken.contains(&slug) {
                    return Err(DomainError::validation(
                        "Another tool server already uses this slug.",
                    )
                    .with_field("slug"));
                }
                slug
            }
            None => Slug::derive(&input.name, &taken),
        };
        let server = ToolServer::create(actor.user_id, &input.name, slug, url, self.clock.now())?
            .with_approval(input.approval);
        let sealed = self.seal(&server, &headers)?;
        let server = server.with_headers(sealed);
        self.servers.insert(&server).await?;
        Ok(server)
    }

    /// Changing the origin (scheme, host, or port) of the URL clears the stored credentials
    /// unless the same update sets new ones, so a token never goes to a host it was not entered
    /// for. Any new URL is listed again before its tools are offered, and every tool asks until
    /// [`ToolService::review`].
    pub async fn update(
        &self,
        actor: &Actor,
        id: ToolServerId,
        changes: ToolServerUpdate,
    ) -> DomainResult<ToolServer> {
        let mut server = self.load(actor, id).await?;
        if let Some(name) = &changes.name {
            server = server.with_name(name)?;
        }
        let mut credentials = changes.credentials;
        if let Some(text) = &changes.url {
            let url = self.url(actor, text)?;
            if !url.same_origin(server.url()) && credentials.is_none() {
                credentials = Some(CredentialChange::Clear);
            }
            server = server.with_url(url);
        }
        match credentials {
            Some(CredentialChange::Set { bearer, headers }) => {
                let headers = ToolHeaders::new(bearer.as_deref(), &headers)?;
                let sealed = self.seal(&server, &headers)?;
                server = server.with_headers(sealed);
            }
            Some(CredentialChange::Clear) => server = server.with_headers(None),
            None => {}
        }
        if let Some(approval) = changes.approval {
            server = server.with_approval(approval);
        }
        if let Some(modes) = changes.tool_modes {
            let listed = |name: &str| {
                server
                    .cache()
                    .is_some_and(|cache| cache.tool(name).is_some())
            };
            if let Some(name) = modes.keys().find(|name| !listed(name)) {
                return Err(DomainError::validation(format!(
                    "{name} is not a tool of this server. Test the server to refresh its tools."
                ))
                .with_field("toolModes"));
            }
            server = server.with_tool_modes(modes);
        }
        if let Some(enabled) = changes.enabled {
            server = server.with_enabled(enabled);
        }
        let server = server.touched(self.clock.now());
        self.servers.update(&server).await?;
        Ok(server)
    }

    /// Deletes the server; projects and chats lose it.
    pub async fn delete(&self, actor: &Actor, id: ToolServerId) -> DomainResult<()> {
        if self.servers.delete(actor.user_id, id).await? {
            Ok(())
        } else {
            Err(not_found(id))
        }
    }

    /// Connects, lists the tools, and stores the list. When any tool's name, description, or
    /// input schema changed since the last list, every tool of the server asks until
    /// [`ToolService::review`]. A URL changed while the list was fetched stores nothing and fails
    /// with `TOOL_SERVER_CHANGED`.
    pub async fn test(&self, actor: &Actor, id: ToolServerId) -> DomainResult<ToolTest> {
        let server = self.load(actor, id).await?;
        let listing = self.fetch(actor, &server).await?;
        let Some((server, changes)) = self.store(actor, server, listing).await? else {
            // Deleted meanwhile (`404`), or its URL changed.
            self.load(actor, id).await?;
            return Err(DomainError::business_rule(
                "The server's URL changed during the test. Test it again.",
                "TOOL_SERVER_CHANGED",
            ));
        };
        Ok(ToolTest { server, changes })
    }

    /// Accepts a changed tool list: the tools follow their modes again.
    pub async fn review(&self, actor: &Actor, id: ToolServerId) -> DomainResult<ToolServer> {
        let server = self
            .load(actor, id)
            .await?
            .reviewed()
            .touched(self.clock.now());
        self.servers.update(&server).await?;
        Ok(server)
    }

    /// The server tools a turn with `selection` may use, sorted by alias. Only enabled servers
    /// of the caller count; `off` tools are dropped. A server whose cached list expired (or that
    /// was never listed at its current URL) is listed again first. When that fails the stale list
    /// is used if it came from the current URL, and any other server is reported in
    /// `unavailable`, as is a server whose URL changed while it was listed.
    pub async fn effective_tools(
        &self,
        actor: &Actor,
        selection: &ToolSelection,
    ) -> DomainResult<EffectiveTools> {
        let ids = selection.servers();
        let mut effective = EffectiveTools::default();
        if ids.is_empty() {
            return Ok(effective);
        }
        let servers = self.servers.list(actor.user_id).await?;
        let now = self.clock.now();
        for id in ids {
            let Some(server) = servers.iter().find(|server| server.id() == id) else {
                continue;
            };
            if !server.enabled() {
                continue;
            }
            let mut server = server.clone();
            if server.needs_refresh(now) {
                match self.fetch(actor, &server).await {
                    Ok(listing) => match self.store(actor, server, listing).await? {
                        Some((stored, _)) => server = stored,
                        None => {
                            effective.unavailable.push(id);
                            continue;
                        }
                    },
                    Err(_) if server.listed() => {}
                    Err(_) => {
                        effective.unavailable.push(id);
                        continue;
                    }
                }
            }
            if let Some(cache) = server.cache() {
                effective.tools.extend(offered(&server, &cache.tools));
            }
        }
        effective
            .tools
            .sort_by(|left, right| left.alias.cmp(&right.alias));
        Ok(effective)
    }

    /// The adapter's view of a server owned by `actor`: the opened credentials and the network
    /// scope. Fails with `SECRET_UNREADABLE` when the credentials no longer open.
    pub fn endpoint(&self, actor: &Actor, server: &ToolServer) -> DomainResult<ToolEndpoint> {
        let headers = server
            .headers_sealed()
            .map(|sealed| self.cipher.open(sealed, &server.headers_context()))
            .transpose()?;
        Ok(server.endpoint(headers, self.network.scope(actor)))
    }

    /// Lists the server's tools: names trimmed, unnamed tools dropped, the first of each name
    /// kept.
    async fn fetch(&self, actor: &Actor, server: &ToolServer) -> DomainResult<ToolListing> {
        let endpoint = self.endpoint(actor, server)?;
        let mut listing = self.host.list(&endpoint).await?;
        let mut seen = HashSet::new();
        listing.tools.retain_mut(|tool| {
            tool.name = tool.name.trim().to_owned();
            !tool.name.is_empty() && seen.insert(tool.name.clone())
        });
        Ok(listing)
    }

    /// Stores a fresh listing of `server`'s URL with its review flag. `None`, with nothing
    /// stored, when the server's URL changed (or the server went away) since it was read.
    async fn store(
        &self,
        actor: &Actor,
        server: ToolServer,
        listing: ToolListing,
    ) -> DomainResult<Option<(ToolServer, ToolChanges)>> {
        let before = server.cache().map(|cache| cache.tools.as_slice());
        let changes = ToolChanges::between(before, &listing.tools);
        let review_required = server.review_required() || !changes.is_empty();
        let now = self.clock.now();
        let saved = self
            .servers
            .save_listing(
                actor.user_id,
                server.id(),
                server.url(),
                &listing,
                review_required,
                now,
            )
            .await?;
        Ok(saved.then(|| (server.with_listing(&listing, review_required, now), changes)))
    }

    async fn load(&self, actor: &Actor, id: ToolServerId) -> DomainResult<ToolServer> {
        self.servers
            .get(actor.user_id, id)
            .await?
            .ok_or_else(|| not_found(id))
    }

    fn url(&self, actor: &Actor, text: &str) -> DomainResult<BaseUrl> {
        BaseUrl::parse(text, self.network.scope(actor)).map_err(|error| error.with_field("url"))
    }

    /// Seals the credentials for `server`'s row; none means nothing stored.
    fn seal(&self, server: &ToolServer, headers: &ToolHeaders) -> DomainResult<Option<Vec<u8>>> {
        if headers.is_empty() {
            return Ok(None);
        }
        self.cipher
            .seal(&headers.to_json(), &server.headers_context())
            .map(Some)
    }
}

fn not_found(id: ToolServerId) -> DomainError {
    DomainError::not_found("Tool server", id.to_string())
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::domains::tool::selection::ServerOverride;
    use crate::ports::tool_host::{RemoteTool, ToolSession};
    use crate::shared::ids::UserId;
    use crate::shared::net::NetworkScope;
    use crate::testing::{
        FakeCipher, FixedClock, MemoryStore, ScriptedToolHost, actor, admin, remote_tool,
    };

    struct Fixture {
        service: ToolService,
        host: Arc<ScriptedToolHost>,
        cipher: Arc<FakeCipher>,
        clock: Arc<FixedClock>,
    }

    fn fixture() -> Fixture {
        let clock = Arc::new(FixedClock::default());
        let store = Arc::new(MemoryStore::new(clock.clone()));
        let host = Arc::new(ScriptedToolHost::new());
        let cipher = Arc::new(FakeCipher::new());
        let service = ToolService::new(
            store,
            host.clone(),
            cipher.clone(),
            clock.clone(),
            PrivateNetwork::Admins,
        );
        Fixture {
            service,
            host,
            cipher,
            clock,
        }
    }

    fn named(name: &str) -> NewToolServer {
        NewToolServer {
            name: name.to_owned(),
            url: "https://mcp.example.com/mcp".to_owned(),
            ..NewToolServer::default()
        }
    }

    fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn opened(f: &Fixture, server: &ToolServer) -> Vec<(String, String)> {
        let sealed = server.headers_sealed().expect("sealed credentials");
        let secret = f.cipher.open(sealed, &server.headers_context()).unwrap();
        ToolHeaders::entries(&secret).unwrap()
    }

    fn described(name: &str, description: &str) -> RemoteTool {
        RemoteTool {
            description: description.to_owned(),
            ..remote_tool(name)
        }
    }

    #[test]
    fn inputs_never_print_credentials() {
        let new = NewToolServer {
            bearer: Some("ghp_TOPSECRET".to_owned()),
            headers: headers(&[("X-Api-Key", "hdr-TOPSECRET")]),
            ..named("GitHub")
        };
        let change = CredentialChange::Set {
            bearer: Some("ghp_TOPSECRET".to_owned()),
            headers: headers(&[("X-Api-Key", "hdr-TOPSECRET")]),
        };
        let update = ToolServerUpdate {
            credentials: Some(change.clone()),
            ..ToolServerUpdate::default()
        };
        for printed in [
            format!("{new:?}"),
            format!("{change:?}"),
            format!("{update:?}"),
        ] {
            assert!(!printed.contains("TOPSECRET"), "{printed}");
            assert!(
                printed.contains("X-Api-Key"),
                "header names stay: {printed}"
            );
        }
    }

    #[tokio::test]
    async fn creates_a_server_with_a_derived_slug_and_sealed_credentials() {
        let f = fixture();
        let caller = actor();
        let input = NewToolServer {
            bearer: Some(" tok-1 ".to_owned()),
            headers: headers(&[("X-Team", "core")]),
            ..named("GitHub Tools")
        };
        let server = f.service.create(&caller, input).await.unwrap();
        assert_eq!(server.slug().as_str(), "githubtools");
        assert_eq!(server.approval(), ApprovalMode::Ask, "new servers ask");
        assert_eq!(
            opened(&f, &server),
            [
                ("Authorization".to_owned(), "Bearer tok-1".to_owned()),
                ("X-Team".to_owned(), "core".to_owned()),
            ]
        );
        let details = f.service.get(&caller, server.id()).await.unwrap();
        assert_eq!(details.credentials, KeyState::Set);
        assert_eq!(
            f.service.list(&caller).await.unwrap(),
            std::slice::from_ref(&server)
        );

        let second = f
            .service
            .create(&caller, named("GitHub Tools"))
            .await
            .unwrap();
        assert_eq!(second.slug().as_str(), "githubtools2");
        assert!(second.headers_sealed().is_none());
        let details = f.service.get(&caller, second.id()).await.unwrap();
        assert_eq!(details.credentials, KeyState::Missing);
    }

    #[tokio::test]
    async fn accepts_a_free_slug_and_rejects_a_taken_or_invalid_one() {
        let f = fixture();
        let caller = actor();
        let input = NewToolServer {
            slug: Some("gh".to_owned()),
            approval: ApprovalMode::Auto,
            ..named("GitHub")
        };
        let server = f.service.create(&caller, input.clone()).await.unwrap();
        assert_eq!(server.slug().as_str(), "gh");
        assert_eq!(server.approval(), ApprovalMode::Auto);
        let error = f.service.create(&caller, input).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "Another tool server already uses this slug."
        );
        let other_user = NewToolServer {
            slug: Some("gh".to_owned()),
            ..named("GitHub")
        };
        assert!(
            f.service.create(&actor(), other_user).await.is_ok(),
            "slugs are unique per user"
        );
        let invalid = NewToolServer {
            slug: Some("Git Hub".to_owned()),
            ..named("GitHub")
        };
        assert!(f.service.create(&caller, invalid).await.is_err());
    }

    #[tokio::test]
    async fn applies_the_network_policy_to_the_url() {
        let f = fixture();
        let private = NewToolServer {
            url: "http://localhost:8080/mcp".to_owned(),
            ..named("Local")
        };
        let error = f
            .service
            .create(&actor(), private.clone())
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "This account cannot connect to private network addresses."
        );
        assert!(
            matches!(error, DomainError::Validation { field: Some(ref field), .. } if field == "url")
        );
        assert!(f.service.create(&admin(), private).await.is_ok());
        let plain = NewToolServer {
            url: "http://mcp.example.com".to_owned(),
            ..named("Plain")
        };
        assert!(f.service.create(&admin(), plain).await.is_err());
    }

    #[tokio::test]
    async fn rejects_a_custom_authorization_header_next_to_a_bearer_token() {
        let f = fixture();
        let input = NewToolServer {
            bearer: Some("tok".to_owned()),
            headers: headers(&[("Authorization", "Token x")]),
            ..named("GitHub")
        };
        let error = f.service.create(&actor(), input).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "Set either a bearer token or a custom Authorization header, not both."
        );
    }

    #[tokio::test]
    async fn a_new_origin_clears_credentials_unless_new_ones_are_given() {
        let f = fixture();
        let caller = actor();
        let input = NewToolServer {
            bearer: Some("tok".to_owned()),
            ..named("GitHub")
        };
        let server = f.service.create(&caller, input).await.unwrap();

        let same_origin = ToolServerUpdate {
            url: Some("https://mcp.example.com/v2".to_owned()),
            ..ToolServerUpdate::default()
        };
        let server = f
            .service
            .update(&caller, server.id(), same_origin)
            .await
            .unwrap();
        assert!(
            server.headers_sealed().is_some(),
            "same origin keeps the token"
        );

        let moved = ToolServerUpdate {
            url: Some("https://other.example.com/mcp".to_owned()),
            ..ToolServerUpdate::default()
        };
        let server = f.service.update(&caller, server.id(), moved).await.unwrap();
        assert!(server.headers_sealed().is_none(), "a new origin clears it");

        let moved_with_token = ToolServerUpdate {
            url: Some("https://third.example.com/mcp".to_owned()),
            credentials: Some(CredentialChange::Set {
                bearer: Some("tok-2".to_owned()),
                headers: BTreeMap::new(),
            }),
            ..ToolServerUpdate::default()
        };
        let server = f
            .service
            .update(&caller, server.id(), moved_with_token)
            .await
            .unwrap();
        assert_eq!(
            opened(&f, &server),
            [("Authorization".to_owned(), "Bearer tok-2".to_owned())]
        );

        let cleared = ToolServerUpdate {
            credentials: Some(CredentialChange::Clear),
            ..ToolServerUpdate::default()
        };
        let server = f
            .service
            .update(&caller, server.id(), cleared)
            .await
            .unwrap();
        assert!(server.headers_sealed().is_none());
    }

    #[tokio::test]
    async fn reports_unreadable_credentials_after_a_key_change() {
        let f = fixture();
        let caller = actor();
        let input = NewToolServer {
            bearer: Some("tok".to_owned()),
            ..named("GitHub")
        };
        let server = f.service.create(&caller, input).await.unwrap();
        f.cipher.rotate();
        let details = f.service.get(&caller, server.id()).await.unwrap();
        assert_eq!(details.credentials, KeyState::Unreadable);
        let error = f.service.test(&caller, server.id()).await.unwrap_err();
        assert_eq!(error.code(), SECRET_UNREADABLE);
    }

    #[tokio::test]
    async fn test_lists_and_stores_the_tools() {
        let f = fixture();
        let caller = admin();
        let input = NewToolServer {
            bearer: Some("tok".to_owned()),
            ..named("GitHub")
        };
        let server = f.service.create(&caller, input).await.unwrap();
        f.host.push_tools(vec![
            remote_tool("search"),
            remote_tool("  "),
            remote_tool(" create_issue "),
            described("search", "A second search"),
        ]);
        let test = f.service.test(&caller, server.id()).await.unwrap();
        assert!(test.changes.is_empty(), "the first list is not a change");
        let cache = test.server.cache().expect("a cached list");
        let names: Vec<&str> = cache.tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(
            names,
            ["search", "create_issue"],
            "trimmed, unnamed and repeated dropped"
        );
        assert_eq!(
            cache.tools[0].description, "The search tool.",
            "the first one wins"
        );
        assert_eq!(cache.fetched_at, f.clock.now());
        assert_eq!(test.server.protocol_version(), Some("2026-07-28"));
        assert!(!test.server.review_required());
        let stored = f.service.get(&caller, server.id()).await.unwrap().server;
        assert_eq!(stored, test.server);

        let endpoint = &f.host.endpoints()[0];
        assert_eq!(endpoint.server_id, server.id());
        assert_eq!(
            endpoint.network,
            NetworkScope::Private,
            "admins may reach private hosts"
        );
        let secret = endpoint.headers.as_ref().expect("opened credentials");
        assert_eq!(
            ToolHeaders::entries(secret).unwrap(),
            [("Authorization".to_owned(), "Bearer tok".to_owned())]
        );
    }

    #[tokio::test]
    async fn a_changed_tool_list_requires_a_review() {
        let f = fixture();
        let caller = actor();
        let server = f.service.create(&caller, named("GitHub")).await.unwrap();
        f.host
            .push_tools(vec![described("search", "Search issues")]);
        f.service.test(&caller, server.id()).await.unwrap();

        f.host.push_tools(vec![
            described("search", "Search issues and delete them"),
            remote_tool("delete"),
        ]);
        let test = f.service.test(&caller, server.id()).await.unwrap();
        assert_eq!(test.changes.changed, ["search"]);
        assert_eq!(test.changes.added, ["delete"]);
        assert!(test.server.review_required());
        assert_eq!(test.server.protocol_version(), Some("2026-07-28"));

        f.host
            .push_tools(test.server.cache().unwrap().tools.clone());
        let again = f.service.test(&caller, server.id()).await.unwrap();
        assert!(again.changes.is_empty());
        assert!(again.server.review_required(), "stays until reviewed");

        let reviewed = f.service.review(&caller, server.id()).await.unwrap();
        assert!(!reviewed.review_required());
        let stored = f.service.get(&caller, server.id()).await.unwrap().server;
        assert!(!stored.review_required());
        assert_eq!(
            stored.cache().unwrap().tools.len(),
            2,
            "a review keeps the list"
        );
    }

    #[tokio::test]
    async fn sets_tool_modes_only_for_listed_tools() {
        let f = fixture();
        let caller = actor();
        let server = f.service.create(&caller, named("GitHub")).await.unwrap();
        let modes = ToolServerUpdate {
            tool_modes: Some(BTreeMap::from([("delete".to_owned(), ToolMode::Off)])),
            ..ToolServerUpdate::default()
        };
        let error = f
            .service
            .update(&caller, server.id(), modes.clone())
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "delete is not a tool of this server. Test the server to refresh its tools."
        );
        f.host.push_tools(vec![remote_tool("delete")]);
        f.service.test(&caller, server.id()).await.unwrap();
        let server = f.service.update(&caller, server.id(), modes).await.unwrap();
        assert_eq!(server.mode("delete"), ToolMode::Off);
        let stored = f.service.get(&caller, server.id()).await.unwrap().server;
        assert_eq!(stored.mode("delete"), ToolMode::Off);
        assert!(stored.cache().is_some(), "an update keeps the cached list");
    }

    #[tokio::test]
    async fn hides_other_users_servers() {
        let f = fixture();
        let alice = actor();
        let bob = actor();
        let server = f.service.create(&alice, named("GitHub")).await.unwrap();
        assert!(f.service.list(&bob).await.unwrap().is_empty());
        for error in [
            f.service.get(&bob, server.id()).await.unwrap_err(),
            f.service
                .update(&bob, server.id(), ToolServerUpdate::default())
                .await
                .unwrap_err(),
            f.service.test(&bob, server.id()).await.unwrap_err(),
            f.service.review(&bob, server.id()).await.unwrap_err(),
            f.service.delete(&bob, server.id()).await.unwrap_err(),
        ] {
            assert!(
                matches!(error, DomainError::EntityNotFound { .. }),
                "{error}"
            );
        }
        assert!(f.host.endpoints().is_empty());
        f.service.delete(&alice, server.id()).await.unwrap();
        assert!(f.service.get(&alice, server.id()).await.is_err());
    }

    /// A server listed with `tools`, returning its id.
    async fn listed(
        f: &Fixture,
        caller: &Actor,
        name: &str,
        tools: Vec<RemoteTool>,
    ) -> ToolServerId {
        let server = f.service.create(caller, named(name)).await.unwrap();
        f.host.push_tools(tools);
        f.service.test(caller, server.id()).await.unwrap();
        server.id()
    }

    fn selection(
        project: Vec<ToolServerId>,
        overrides: Vec<(ToolServerId, bool)>,
    ) -> ToolSelection {
        ToolSelection {
            project,
            overrides: overrides
                .into_iter()
                .map(|(server_id, enabled)| ServerOverride { server_id, enabled })
                .collect(),
        }
    }

    fn aliases(tools: &EffectiveTools) -> Vec<&str> {
        tools.tools.iter().map(|tool| tool.alias.as_str()).collect()
    }

    #[tokio::test]
    async fn effective_tools_follow_the_selection_modes_and_enabled_flags() {
        let f = fixture();
        let caller = actor();
        let github = listed(
            &f,
            &caller,
            "GitHub",
            vec![remote_tool("search"), remote_tool("delete")],
        )
        .await;
        let files = listed(&f, &caller, "Files", vec![remote_tool("read")]).await;
        let mail = listed(&f, &caller, "Mail", vec![remote_tool("send")]).await;
        let off = ToolServerUpdate {
            tool_modes: Some(BTreeMap::from([("delete".to_owned(), ToolMode::Off)])),
            approval: Some(ApprovalMode::Auto),
            ..ToolServerUpdate::default()
        };
        f.service.update(&caller, github, off).await.unwrap();
        let disabled = ToolServerUpdate {
            enabled: Some(false),
            ..ToolServerUpdate::default()
        };
        f.service.update(&caller, mail, disabled).await.unwrap();
        let calls = f.host.endpoints().len();

        let effective = f
            .service
            .effective_tools(
                &caller,
                &selection(vec![github, files], vec![(mail, true), (files, false)]),
            )
            .await
            .unwrap();
        assert_eq!(aliases(&effective), ["github__search"]);
        assert_eq!(effective.tools[0].mode, ToolMode::Auto);
        assert_eq!(effective.tools[0].name, "search");
        assert!(effective.unavailable.is_empty());

        let effective = f
            .service
            .effective_tools(&caller, &selection(vec![files], vec![(github, true)]))
            .await
            .unwrap();
        assert_eq!(
            aliases(&effective),
            ["files__read", "github__search"],
            "sorted by alias"
        );
        assert_eq!(effective.tools[0].mode, ToolMode::Ask);
        assert_eq!(
            f.host.endpoints().len(),
            calls,
            "fresh caches do not connect"
        );

        let other = f
            .service
            .effective_tools(&actor(), &selection(vec![github], vec![]))
            .await
            .unwrap();
        assert!(other.tools.is_empty(), "other users' servers never count");
    }

    #[tokio::test]
    async fn effective_tools_refresh_expired_lists() {
        let f = fixture();
        let caller = actor();
        let github = listed(&f, &caller, "GitHub", vec![remote_tool("search")]).await;
        f.clock.advance(Duration::hours(1));
        f.host
            .push_tools(vec![remote_tool("search"), remote_tool("create")]);
        let effective = f
            .service
            .effective_tools(&caller, &selection(vec![github], vec![]))
            .await
            .unwrap();
        assert_eq!(aliases(&effective), ["github__create", "github__search"]);
        assert!(
            effective.tools.iter().all(|tool| tool.review_required),
            "an added tool asks"
        );
        let stored = f.service.get(&caller, github).await.unwrap().server;
        assert_eq!(stored.cache().unwrap().fetched_at, f.clock.now());
    }

    #[tokio::test]
    async fn effective_tools_fall_back_to_a_stale_list_or_report_the_server() {
        let f = fixture();
        let caller = actor();
        let github = listed(&f, &caller, "GitHub", vec![remote_tool("search")]).await;
        let never = f
            .service
            .create(&caller, named("Never"))
            .await
            .unwrap()
            .id();
        f.clock.advance(Duration::hours(2));
        f.host
            .push_error(DomainError::business_rule("down", "DOWN"));
        f.host
            .push_error(DomainError::business_rule("down", "DOWN"));
        let effective = f
            .service
            .effective_tools(&caller, &selection(vec![github, never], vec![]))
            .await
            .unwrap();
        assert_eq!(
            aliases(&effective),
            ["github__search"],
            "the stale list is used"
        );
        assert_eq!(effective.unavailable, [never]);
        assert!(
            f.service
                .effective_tools(&caller, &ToolSelection::default())
                .await
                .unwrap()
                .tools
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_new_url_is_listed_again_and_every_tool_asks_until_a_review() {
        let f = fixture();
        let caller = actor();
        let github = listed(&f, &caller, "GitHub", vec![remote_tool("search")]).await;
        f.service
            .update(
                &caller,
                github,
                ToolServerUpdate {
                    approval: Some(ApprovalMode::Auto),
                    ..ToolServerUpdate::default()
                },
            )
            .await
            .unwrap();
        let moved = f
            .service
            .update(
                &caller,
                github,
                ToolServerUpdate {
                    url: Some("https://other-host.example.net/mcp".to_owned()),
                    ..ToolServerUpdate::default()
                },
            )
            .await
            .unwrap();
        assert!(moved.review_required());

        f.host
            .push_error(DomainError::business_rule("down", "DOWN"));
        let effective = f
            .service
            .effective_tools(&caller, &selection(vec![github], vec![]))
            .await
            .unwrap();
        assert!(
            effective.tools.is_empty(),
            "the old URL's list is never offered for the new URL"
        );
        assert_eq!(effective.unavailable, [github]);

        f.host.push_tools(vec![remote_tool("search")]);
        let effective = f
            .service
            .effective_tools(&caller, &selection(vec![github], vec![]))
            .await
            .unwrap();
        let endpoints = f.host.endpoints();
        assert_eq!(
            endpoints.last().unwrap().url.as_str(),
            "https://other-host.example.net/mcp"
        );
        assert_eq!(aliases(&effective), ["github__search"]);
        assert!(
            effective.tools.iter().all(|tool| tool.review_required),
            "the same tools still ask on a new host"
        );

        f.service.review(&caller, github).await.unwrap();
        let effective = f
            .service
            .effective_tools(&caller, &selection(vec![github], vec![]))
            .await
            .unwrap();
        assert_eq!(effective.tools[0].mode, ToolMode::Auto);
        assert!(!effective.tools[0].review_required);
    }

    /// The scripted host, with the user moving the server to a new URL while it lists
    /// (`https://moved-<n>.example.net/mcp` on the n-th list).
    struct Moving {
        host: Arc<ScriptedToolHost>,
        store: Arc<MemoryStore>,
        owner: UserId,
        moves: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl ToolHost for Moving {
        async fn list(&self, endpoint: &ToolEndpoint) -> DomainResult<ToolListing> {
            let stored =
                ToolServerRepository::get(self.store.as_ref(), self.owner, endpoint.server_id)
                    .await?
                    .expect("the server");
            let moves = self.moves.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            let url = BaseUrl::restore(&format!("https://moved-{moves}.example.net/mcp"))?;
            ToolServerRepository::update(self.store.as_ref(), &stored.with_url(url)).await?;
            self.host.list(endpoint).await
        }

        async fn open(&self, endpoints: Vec<ToolEndpoint>) -> DomainResult<Box<dyn ToolSession>> {
            self.host.open(endpoints).await
        }
    }

    #[tokio::test]
    async fn a_list_from_a_url_changed_meanwhile_is_never_stored() {
        let clock = Arc::new(FixedClock::default());
        let store = Arc::new(MemoryStore::new(clock.clone()));
        let host = Arc::new(ScriptedToolHost::new());
        let caller = actor();
        let moving = Arc::new(Moving {
            host: host.clone(),
            store: store.clone(),
            owner: caller.user_id,
            moves: std::sync::atomic::AtomicUsize::new(0),
        });
        let service = ToolService::new(
            store.clone(),
            moving,
            Arc::new(FakeCipher::new()),
            clock,
            PrivateNetwork::Admins,
        );
        let server = service.create(&caller, named("GitHub")).await.unwrap();

        host.push_tools(vec![remote_tool("search")]);
        let error = service.test(&caller, server.id()).await.unwrap_err();
        assert_eq!(error.code(), "TOOL_SERVER_CHANGED");
        let stored = service.get(&caller, server.id()).await.unwrap().server;
        assert_eq!(stored.url().as_str(), "https://moved-1.example.net/mcp");
        assert!(stored.cache().is_none(), "the old URL's list is not stored");
        assert!(!stored.listed());

        host.push_tools(vec![remote_tool("search")]);
        let effective = service
            .effective_tools(&caller, &selection(vec![server.id()], vec![]))
            .await
            .unwrap();
        assert!(effective.tools.is_empty(), "nothing from the old URL");
        assert_eq!(effective.unavailable, [server.id()]);
        let stored = service.get(&caller, server.id()).await.unwrap().server;
        assert_eq!(stored.url().as_str(), "https://moved-2.example.net/mcp");
        assert!(stored.cache().is_none());
    }
}
