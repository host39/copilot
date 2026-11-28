//! Tool server use cases: settings, tests (tool listing and change detection), reviews, the
//! tools a turn gets, OAuth sign-in (`sign_in`), and the rules of stdio servers (`stdio`):
//! administrators only, while `MCP_STDIO_ENABLED` is on, checked on save and before every start.

mod sign_in;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{DateTime, Duration, Utc};
use futures_util::stream::{self, StreamExt, TryStreamExt};

use crate::domains::provider::KeyState;
use crate::domains::tool::auth::{AuthMode, ClientConfig, SignInState};
use crate::domains::tool::entities::{Location, ToolServer};
use crate::domains::tool::headers::ToolHeaders;
use crate::domains::tool::ports::ToolServerRepository;
use crate::domains::tool::selection::{
    EffectiveTools, ToolChanges, ToolSelection, UnavailableServer, offered,
};
use crate::domains::tool::stdio::{
    self, StdioCommand, StdioEnv, StdioGate, StdioInput, StdioPolicy, StdioUpdate, Transport,
};
use crate::domains::tool::values::{ApprovalMode, Slug, ToolMode};
use crate::ports::clock::Clock;
use crate::ports::secret_cipher::SecretCipher;
use crate::ports::tool_host::{
    SpawnGate, StdioLaunch, Target, ToolEndpoint, ToolHost, ToolListing,
};
use crate::shared::actor::Actor;
use crate::shared::base_url::BaseUrl;
use crate::shared::constants::{
    MAX_PARALLEL_READS, OAUTH_CLIENT_MAX_CHARS, TOOL_REFRESH_BACKOFF_SECS,
};
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::ToolServerId;
use crate::shared::net::PrivateNetwork;
use crate::shared::secret::REDACTED;

use crate::domains::tool::service::sign_in::OAuth;
pub use crate::domains::tool::service::sign_in::{
    CLIENT_ID_REQUIRED, CLIENT_ISSUER_CHANGED, CLIENT_NAME, Callback, SIGN_IN_UNAVAILABLE,
    SignInConfig, SignInOutcome, SignInResult, SignInStart,
};

/// Input of [`ToolService::create`]. `Debug` prints `***` for the token, header, and
/// environment values.
#[derive(Clone, Default)]
pub struct NewToolServer {
    pub name: String,
    /// `None` derives a free slug from the name.
    pub slug: Option<String>,
    /// `Stdio` needs `stdio` and no URL or credentials.
    pub transport: Transport,
    /// Blank for a stdio server.
    pub url: String,
    /// The program of a stdio server.
    pub stdio: Option<StdioInput>,
    /// Blank means none.
    pub bearer: Option<String>,
    pub headers: BTreeMap<String, String>,
    /// `None` is `Headers` when a bearer token or headers are given, else `None`.
    pub auth: Option<AuthMode>,
    /// A pre-registered OAuth client (`OAuth` only).
    pub client: Option<ClientInput>,
    pub approval: ApprovalMode,
}

impl fmt::Debug for NewToolServer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NewToolServer")
            .field("name", &self.name)
            .field("slug", &self.slug)
            .field("transport", &self.transport)
            .field("url", &self.url)
            .field("stdio", &self.stdio)
            .field("bearer", &self.bearer.as_ref().map(|_| REDACTED))
            .field("headers", &Redacted(&self.headers))
            .field("auth", &self.auth)
            .field("client", &self.client)
            .field("approval", &self.approval)
            .finish()
    }
}

/// A pre-registered OAuth client as the user enters it. `Debug` hides the secret.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct ClientInput {
    pub id: String,
    /// A confidential client's secret. Blank or absent keeps the stored secret while the id
    /// stays the same, and means none for a new id.
    pub secret: Option<String>,
}

impl fmt::Debug for ClientInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientInput")
            .field("id", &self.id)
            .field("secret", &self.secret.as_ref().map(|_| REDACTED))
            .finish()
    }
}

/// A change to the pre-registered OAuth client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientChange {
    Set(ClientInput),
    Clear,
}

/// Input of [`ToolService::update`]; `None` keeps a field. The slug and the transport never
/// change.
#[derive(Clone, Debug, Default)]
pub struct ToolServerUpdate {
    pub name: Option<String>,
    /// Streamable HTTP servers only.
    pub url: Option<String>,
    /// stdio servers only; administrators only.
    pub stdio: Option<StdioUpdate>,
    pub credentials: Option<CredentialChange>,
    /// Switching modes drops the other modes' credentials; `Headers` needs headers (given now or
    /// stored).
    pub auth: Option<AuthMode>,
    pub client: Option<ClientChange>,
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
    /// The bearer token and headers.
    pub credentials: KeyState,
    /// The pre-registered client's secret.
    pub client_secret: KeyState,
    /// The OAuth sign-in (`None` outside `OAuth`); a sign-in that no longer opens needs a new
    /// one.
    pub sign_in: Option<SignInState>,
    /// A stdio server's environment.
    pub env: KeyState,
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
    /// When a turn last failed to list each server: no turn lists it again before
    /// `TOOL_REFRESH_BACKOFF_SECS` have passed. Kept in memory (one server process).
    failures: Arc<Mutex<HashMap<ToolServerId, DateTime<Utc>>>>,
    /// OAuth sign-in ([`ToolService::with_sign_in`]).
    oauth: Option<Arc<OAuth>>,
    /// stdio servers ([`ToolService::with_stdio`]); `None` is `MCP_STDIO_ENABLED` off.
    stdio: Option<StdioPolicy>,
}

/// What a turn's refresh of one server came to.
enum Refresh {
    /// Fresh, or not due: the cached list is current.
    Current,
    /// Listing failed now or recently: the stale list is used when it came from the current URL.
    Failed,
    /// The URL changed (or the server went) while it was listed.
    Moved,
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
            failures: Arc::default(),
            oauth: None,
            stdio: None,
        }
    }

    /// Lets administrators add and run stdio servers when `policy.enabled`
    /// (`MCP_STDIO_ENABLED`); `policy.accounts` tells, before every start, whether the owner is
    /// still an administrator.
    #[must_use]
    pub fn with_stdio(mut self, policy: StdioPolicy) -> Self {
        self.stdio = Some(policy);
        self
    }

    /// Whether `actor` may add and run stdio servers: the setting is on and the session's
    /// account is an administrator.
    pub fn stdio_available(&self, actor: &Actor) -> bool {
        self.stdio
            .as_ref()
            .is_some_and(|policy| policy.allows(actor))
    }

    /// Fails unless [`ToolService::stdio_available`], with the reason.
    fn stdio_check(&self, actor: &Actor) -> DomainResult<&StdioPolicy> {
        let policy = self.stdio.as_ref().ok_or_else(stdio::disabled)?;
        policy.check(actor)?;
        Ok(policy)
    }

    pub async fn list(&self, actor: &Actor) -> DomainResult<Vec<ToolServer>> {
        self.servers.list(actor.user_id).await
    }

    pub async fn get(&self, actor: &Actor, id: ToolServerId) -> DomainResult<ToolServerDetails> {
        let server = self.load(actor, id).await?;
        let credentials = KeyState::check(
            &*self.cipher,
            server.headers_sealed(),
            &server.headers_context(),
        )?;
        let client_secret = KeyState::check(
            &*self.cipher,
            server
                .client()
                .and_then(|client| client.secret_sealed.as_deref()),
            &server.client_context(),
        )?;
        let readable = KeyState::check(
            &*self.cipher,
            server.sign_in().map(|sign_in| sign_in.sealed.as_slice()),
            &server.sign_in_context(),
        )?;
        let sign_in = match self.sign_in_state(&server) {
            Some(_) if readable == KeyState::Unreadable => Some(SignInState::NeedsSignIn),
            state => state,
        };
        let env = KeyState::check(
            &*self.cipher,
            server
                .stdio()
                .and_then(|command| command.env_sealed.as_deref()),
            &server.env_context(),
        )?;
        Ok(ToolServerDetails {
            server,
            credentials,
            client_secret,
            sign_in,
            env,
        })
    }

    /// The names of a stdio server's environment variables (never their values); empty when it
    /// has none or they no longer open.
    pub fn env_names(&self, server: &ToolServer) -> Vec<String> {
        server
            .stdio()
            .and_then(|command| command.env_sealed.as_deref())
            .and_then(|sealed| self.cipher.open(sealed, &server.env_context()).ok())
            .and_then(|secret| StdioEnv::names(&secret).ok())
            .unwrap_or_default()
    }

    /// What the settings show for an `OAuth` server now; `None` in the other modes.
    pub fn sign_in_state(&self, server: &ToolServer) -> Option<SignInState> {
        server.sign_in_state(self.clock.now())
    }

    /// Registers a server. It has no tools until the first test. A stdio server needs an
    /// administrator and `MCP_STDIO_ENABLED`.
    pub async fn create(&self, actor: &Actor, input: NewToolServer) -> DomainResult<ToolServer> {
        if input.transport == Transport::Stdio {
            return self.create_stdio(actor, input).await;
        }
        if input.stdio.is_some() {
            return Err(only_stdio());
        }
        let url = self.url(actor, &input.url)?;
        let headers = ToolHeaders::new(input.bearer.as_deref(), &input.headers)?;
        let mode = input.auth.unwrap_or(if headers.is_empty() {
            AuthMode::None
        } else {
            AuthMode::Headers
        });
        check_mode(mode, &headers)?;
        if input.client.is_some() && mode != AuthMode::OAuth {
            return Err(only_oauth());
        }
        let slug = self.slug(actor, &input).await?;
        let server = ToolServer::create(actor.user_id, &input.name, slug, url, self.clock.now())?
            .with_approval(input.approval);
        let sealed = self.seal(&server, &headers)?;
        let mut server = server.with_headers(sealed).with_auth(mode);
        if let Some(input) = &input.client {
            let client = self.client(&server, input)?;
            server = server.with_client(Some(client));
        }
        self.servers.insert(&server).await?;
        Ok(server)
    }

    /// A stdio server: its program, arguments, environment (sealed), and directory, without a
    /// URL or credentials.
    async fn create_stdio(&self, actor: &Actor, input: NewToolServer) -> DomainResult<ToolServer> {
        self.stdio_check(actor)?;
        if !input.url.trim().is_empty() {
            return Err(
                DomainError::validation("A stdio server runs a program and has no URL.")
                    .with_field("url"),
            );
        }
        let has_credentials = input
            .bearer
            .as_deref()
            .is_some_and(|bearer| !bearer.trim().is_empty())
            || !input.headers.is_empty()
            || input.auth.is_some_and(|mode| mode != AuthMode::None)
            || input.client.is_some();
        if has_credentials {
            return Err(no_credentials());
        }
        let Some(program) = &input.stdio else {
            return Err(
                DomainError::validation("Enter the program's path.").with_field("stdio.command")
            );
        };
        let command = StdioCommand {
            command: stdio::command(&program.command)?,
            args: stdio::args(&program.args)?,
            env_sealed: None,
            cwd: stdio::cwd(program.cwd.as_deref(), "")?,
        };
        let env = StdioEnv::new(&program.env)?;
        let slug = self.slug(actor, &input).await?;
        let server =
            ToolServer::create_stdio(actor.user_id, &input.name, slug, command, self.clock.now())?
                .with_approval(input.approval);
        let sealed = self.seal_env(&server, &env)?;
        let server = server.with_env(sealed);
        self.servers.insert(&server).await?;
        tracing::info!(server_id = %server.id(), user_id = %actor.user_id, "core.tools.stdio.created");
        Ok(server)
    }

    /// The slug the input asks for, or a free one derived from the name.
    async fn slug(&self, actor: &Actor, input: &NewToolServer) -> DomainResult<Slug> {
        let taken: Vec<Slug> = self
            .servers
            .list(actor.user_id)
            .await?
            .iter()
            .map(|server| server.slug().clone())
            .collect();
        match input.slug.as_deref() {
            Some(text) => {
                let slug = Slug::parse(text)?;
                if taken.contains(&slug) {
                    return Err(DomainError::validation(
                        "Another tool server already uses this slug.",
                    )
                    .with_field("slug"));
                }
                Ok(slug)
            }
            None => Ok(Slug::derive(&input.name, &taken)),
        }
    }

    /// Changing the origin (scheme, host, or port) of the URL clears the stored credentials
    /// unless the same update sets new ones, so a token never goes to a host it was not entered
    /// for; any new URL ends the OAuth sign-in, whose tokens were issued for the old one. Any new
    /// URL is listed again before its tools are offered, and every tool asks until
    /// [`ToolService::review`]. Credentials given without an auth mode switch to `Headers`;
    /// another mode drops the credentials of the old one. A stdio server's program changes only
    /// for an administrator while `MCP_STDIO_ENABLED` is on, and like a new URL it is listed and
    /// reviewed again; it never takes a URL or credentials.
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
        let program = changes.stdio.as_ref().filter(|update| !update.is_empty());
        match server.transport() {
            Transport::Http if program.is_some() => return Err(only_stdio()),
            Transport::Http => {}
            Transport::Stdio => {
                if changes.url.is_some() {
                    return Err(DomainError::validation(
                        "A stdio server runs a program and has no URL.",
                    )
                    .with_field("url"));
                }
                if changes.auth.is_some_and(|mode| mode != AuthMode::None) {
                    return Err(no_credentials());
                }
                if let Some(update) = program {
                    server = self.reprogram(actor, server, update)?;
                }
            }
        }
        let mut credentials = changes.credentials;
        let mut client_change = changes.client;
        if let Some(text) = &changes.url {
            let url = self.url(actor, text)?;
            // Credentials entered for one origin never follow the URL to another: the headers,
            // and the pre-registered client with its secret, unless this update sets new ones.
            if server.url().is_none_or(|current| !url.same_origin(current)) {
                if credentials.is_none() {
                    credentials = Some(CredentialChange::Clear);
                }
                if client_change.is_none() && server.client().is_some() {
                    client_change = Some(ClientChange::Clear);
                }
            }
            server = server.with_url(url);
        }
        let headers = match &credentials {
            Some(CredentialChange::Set { bearer, headers }) => {
                Some(ToolHeaders::new(bearer.as_deref(), headers)?)
            }
            Some(CredentialChange::Clear) => Some(ToolHeaders::default()),
            None => None,
        };
        if let Some(mode) = changes.auth {
            match &headers {
                Some(headers) => check_mode(mode, headers)?,
                None if mode == AuthMode::Headers && server.headers_sealed().is_none() => {
                    return Err(missing_headers());
                }
                None => {}
            }
        }
        if let Some(headers) = headers {
            if server.transport() == Transport::Stdio && !headers.is_empty() {
                return Err(no_credentials());
            }
            let sealed = self.seal(&server, &headers)?;
            server = server.with_headers(sealed);
        }
        if let Some(mode) = changes.auth {
            server = server.with_auth(mode);
        }
        match &client_change {
            Some(ClientChange::Set(input)) => {
                if server.auth() != AuthMode::OAuth {
                    return Err(only_oauth());
                }
                let client = self.client(&server, input)?;
                server = server.with_client(Some(client));
            }
            Some(ClientChange::Clear) => server = server.with_client(None),
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
        // New settings may fix what failed: the next turn lists the server again.
        self.failures().remove(&id);
        Ok(server)
    }

    /// A stdio server with `update` applied: the policy first, then each field checked. An
    /// environment equal to the stored one keeps its sealed bytes, so saving the form unchanged
    /// does not count as a new program.
    fn reprogram(
        &self,
        actor: &Actor,
        server: ToolServer,
        update: &StdioUpdate,
    ) -> DomainResult<ToolServer> {
        self.stdio_check(actor)?;
        let Some(current) = server.stdio().cloned() else {
            return Err(only_stdio());
        };
        let command = match &update.command {
            Some(text) => stdio::command(text)?,
            None => current.command.clone(),
        };
        let args = match &update.args {
            Some(list) => stdio::args(list)?,
            None => current.args.clone(),
        };
        let cwd = match &update.cwd {
            Some(text) => stdio::cwd(Some(text), &server.id().to_string())?,
            None => current.cwd.clone(),
        };
        let env_sealed = match &update.env {
            Some(variables) => {
                let env = StdioEnv::new(variables)?;
                let stored = current
                    .env_sealed
                    .as_deref()
                    .and_then(|sealed| self.cipher.open(sealed, &server.env_context()).ok());
                let same = match &stored {
                    Some(secret) => secret.expose() == env.to_json().as_slice(),
                    None => env.is_empty(),
                };
                if same {
                    current.env_sealed.clone()
                } else {
                    self.seal_env(&server, &env)?
                }
            }
            None => current.env_sealed.clone(),
        };
        tracing::info!(server_id = %server.id(), user_id = %actor.user_id, "core.tools.stdio.changed");
        Ok(server.with_stdio(StdioCommand {
            command,
            args,
            env_sealed,
            cwd,
        }))
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
        let transport = server.transport();
        let listing = self.fetch(actor, &server).await?;
        self.failures().remove(&id);
        let Some((server, changes)) = self.store(actor, server, listing).await? else {
            // Deleted meanwhile (`404`), or its URL changed.
            self.load(actor, id).await?;
            let changed = match transport {
                Transport::Http => "The server's URL changed during the test. Test it again.",
                Transport::Stdio => "The server's program changed during the test. Test it again.",
            };
            return Err(DomainError::business_rule(changed, "TOOL_SERVER_CHANGED"));
        };
        Ok(ToolTest { server, changes })
    }

    /// A call named a tool the server no longer lists (`TOOL_UNKNOWN`): the stored list expires
    /// now, so the next turn lists the server again.
    pub async fn stale(&self, actor: &Actor, id: ToolServerId) -> DomainResult<()> {
        self.servers.stale(actor.user_id, id).await
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
    /// was never listed at its current URL) is listed again first, at most `MAX_PARALLEL_READS`
    /// at once, unless a listing of it failed less than `TOOL_REFRESH_BACKOFF_SECS` ago. When
    /// listing fails the stale list is used if it came from the current URL; any other server is
    /// reported in `unavailable`, as is a server whose URL changed while it was listed.
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
        let (chosen, blocked): (Vec<ToolServer>, Vec<ToolServer>) = ids
            .iter()
            .filter_map(|id| servers.iter().find(|server| server.id() == *id))
            .filter(|server| server.enabled())
            .cloned()
            .partition(|server| {
                server.transport() == Transport::Http || self.stdio_available(actor)
            });
        // A stdio server runs only for an administrator while stdio is on: never listed or
        // offered otherwise.
        effective
            .unavailable
            .extend(blocked.iter().map(|server| UnavailableServer {
                id: server.id(),
                name: server.name().to_owned(),
            }));
        let refreshed: Vec<(ToolServer, Refresh)> = stream::iter(chosen)
            .map(|server| self.refresh(actor, server, now))
            .buffered(MAX_PARALLEL_READS)
            .try_collect()
            .await?;
        for (server, refresh) in refreshed {
            let usable = match refresh {
                Refresh::Current => true,
                Refresh::Failed => server.listed(),
                Refresh::Moved => false,
            };
            if !usable {
                effective.unavailable.push(UnavailableServer {
                    id: server.id(),
                    name: server.name().to_owned(),
                });
                continue;
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

    /// Lists `server` again when its cache is due and no listing of it failed recently.
    async fn refresh(
        &self,
        actor: &Actor,
        server: ToolServer,
        now: DateTime<Utc>,
    ) -> DomainResult<(ToolServer, Refresh)> {
        if !server.needs_refresh(now) {
            return Ok((server, Refresh::Current));
        }
        let backoff = Duration::seconds(TOOL_REFRESH_BACKOFF_SECS);
        let failed = self.failures().get(&server.id()).copied();
        if failed.is_some_and(|at| now < at + backoff) {
            return Ok((server, Refresh::Failed));
        }
        match self.fetch(actor, &server).await {
            Ok(listing) => {
                self.failures().remove(&server.id());
                Ok(match self.store(actor, server.clone(), listing).await? {
                    Some((stored, _)) => (stored, Refresh::Current),
                    None => (server, Refresh::Moved),
                })
            }
            Err(error) => {
                tracing::warn!(
                    server_id = %server.id(),
                    code = error.code(),
                    "core.tools.refresh.failed"
                );
                self.failures().insert(server.id(), now);
                Ok((server, Refresh::Failed))
            }
        }
    }

    fn failures(&self) -> std::sync::MutexGuard<'_, HashMap<ToolServerId, DateTime<Utc>>> {
        self.failures.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The adapter's view of a server owned by `actor`: the opened credentials and the network
    /// scope. Fails with `SECRET_UNREADABLE` when the credentials no longer open.
    ///
    /// A stdio server needs an administrator and `MCP_STDIO_ENABLED` now, and its launch carries
    /// a gate that checks both again, with the owner's account as stored, right before every
    /// start of its process.
    pub fn endpoint(&self, actor: &Actor, server: &ToolServer) -> DomainResult<ToolEndpoint> {
        let network = self.network.scope(actor);
        let url = match server.location() {
            Location::Http(url) => url.clone(),
            Location::Stdio(command) => {
                let policy = self.stdio_check(actor)?;
                let env = command
                    .env_sealed
                    .as_deref()
                    .map(|sealed| self.cipher.open(sealed, &server.env_context()))
                    .transpose()?;
                let gate = StdioGate {
                    policy: policy.clone(),
                    user: server.user_id(),
                    server: server.id(),
                };
                let launch = StdioLaunch {
                    command: command.command.clone(),
                    args: command.args.clone(),
                    env,
                    cwd: command.cwd.clone(),
                    gate: SpawnGate(Arc::new(gate)),
                };
                return Ok(server.endpoint(Target::Stdio(launch), None, None, network));
            }
        };
        if server.auth() == AuthMode::OAuth {
            let tokens = self.tokens(actor, server)?;
            return Ok(server.endpoint(Target::Http(url), None, Some(tokens), network));
        }
        let headers = server
            .headers_sealed()
            .map(|sealed| self.cipher.open(sealed, &server.headers_context()))
            .transpose()?;
        Ok(server.endpoint(Target::Http(url), headers, None, network))
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
                server.location(),
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

    /// The pre-registered client to store: the id trimmed, the secret sealed for `server`'s row.
    /// A blank secret keeps the stored one while the id stays the same.
    fn client(&self, server: &ToolServer, input: &ClientInput) -> DomainResult<ClientConfig> {
        let id = input.id.trim();
        if id.is_empty() {
            return Err(
                DomainError::validation("Enter the OAuth client ID.").with_field("oauthClient")
            );
        }
        if id.chars().count() > OAUTH_CLIENT_MAX_CHARS
            || id
                .chars()
                .any(|character| character.is_whitespace() || character.is_control())
        {
            return Err(DomainError::validation(format!(
                "The OAuth client ID must be one word of at most {OAUTH_CLIENT_MAX_CHARS} characters."
            ))
            .with_field("oauthClient"));
        }
        let secret = input
            .secret
            .as_deref()
            .map(str::trim)
            .filter(|secret| !secret.is_empty());
        let secret_sealed = match secret {
            Some(secret) => {
                if secret.chars().count() > OAUTH_CLIENT_MAX_CHARS
                    || secret.chars().any(char::is_control)
                {
                    return Err(DomainError::validation(format!(
                        "The OAuth client secret must be one line of at most {OAUTH_CLIENT_MAX_CHARS} characters."
                    ))
                    .with_field("oauthClient"));
                }
                Some(
                    self.cipher
                        .seal(secret.as_bytes(), &server.client_context())?,
                )
            }
            None => server
                .client()
                .filter(|stored| stored.id == id)
                .and_then(|stored| stored.secret_sealed.clone()),
        };
        // Saved again, the client is bound to an authorization server anew at its next sign-in.
        Ok(ClientConfig {
            id: id.to_owned(),
            secret_sealed,
            issuer: None,
        })
    }

    /// Seals a stdio server's environment for its row; none means nothing stored.
    fn seal_env(&self, server: &ToolServer, env: &StdioEnv) -> DomainResult<Option<Vec<u8>>> {
        if env.is_empty() {
            return Ok(None);
        }
        self.cipher
            .seal(&env.to_json(), &server.env_context())
            .map(Some)
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

fn only_stdio() -> DomainError {
    DomainError::validation("A program, arguments, and environment are only for stdio servers.")
        .with_field("stdio")
}

fn no_credentials() -> DomainError {
    DomainError::validation(
        "A stdio server takes no bearer token, headers, or sign-in: give its program the credentials it needs as environment variables.",
    )
    .with_field("authMode")
}

fn not_found(id: ToolServerId) -> DomainError {
    DomainError::not_found("Tool server", id.to_string())
}

/// Headers go with `Headers` only, and `Headers` needs some.
fn check_mode(mode: AuthMode, headers: &ToolHeaders) -> DomainResult<()> {
    match mode {
        AuthMode::Headers if headers.is_empty() => Err(missing_headers()),
        AuthMode::None | AuthMode::OAuth if !headers.is_empty() => Err(DomainError::validation(
            "A bearer token or headers need the \"Bearer token or headers\" authentication.",
        )
        .with_field("authMode")),
        _ => Ok(()),
    }
}

fn missing_headers() -> DomainError {
    DomainError::validation("Enter a bearer token or at least one header.").with_field("bearer")
}

fn only_oauth() -> DomainError {
    DomainError::validation("A client ID is only used with OAuth sign-in.")
        .with_field("oauthClient")
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::domains::tool::selection::ServerOverride;
    use crate::domains::tool::stdio::STDIO_DISABLED;
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

    fn unreachable(effective: &EffectiveTools) -> Vec<ToolServerId> {
        effective
            .unavailable
            .iter()
            .map(|server| server.id)
            .collect()
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
        assert_eq!(error.code(), crate::ports::secret_cipher::SECRET_UNREADABLE);
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
        assert_eq!(unreachable(&effective), [never]);
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
    async fn a_server_that_failed_is_listed_again_only_after_the_back_off() {
        let f = fixture();
        let caller = actor();
        let github = listed(&f, &caller, "GitHub", vec![remote_tool("search")]).await;
        let never = f
            .service
            .create(&caller, named("Never"))
            .await
            .unwrap()
            .id();
        let both = selection(vec![github, never], vec![]);
        f.clock.advance(chrono::Duration::days(2));
        let tries = f.host.endpoints().len();

        f.host
            .push_error(DomainError::business_rule("down", "DOWN"));
        f.host
            .push_error(DomainError::business_rule("down", "DOWN"));
        let effective = f.service.effective_tools(&caller, &both).await.unwrap();
        assert_eq!(f.host.endpoints().len(), tries + 2, "both listed at once");
        assert_eq!(aliases(&effective), ["github__search"], "the stale list");
        assert_eq!(unreachable(&effective), [never], "never listed");

        f.clock
            .advance(chrono::Duration::seconds(TOOL_REFRESH_BACKOFF_SECS - 1));
        let effective = f.service.effective_tools(&caller, &both).await.unwrap();
        assert_eq!(
            f.host.endpoints().len(),
            tries + 2,
            "no turn waits on a server that just failed"
        );
        assert_eq!(aliases(&effective), ["github__search"]);
        assert_eq!(unreachable(&effective), [never]);

        f.service
            .update(
                &caller,
                never,
                ToolServerUpdate {
                    name: Some("Renamed".to_owned()),
                    ..ToolServerUpdate::default()
                },
            )
            .await
            .unwrap();
        f.host.push_tools(vec![remote_tool("read")]);
        let effective = f
            .service
            .effective_tools(&caller, &selection(vec![never], vec![]))
            .await
            .unwrap();
        assert_eq!(
            f.host.endpoints().len(),
            tries + 3,
            "a settings change lists it again at once"
        );
        assert!(effective.unavailable.is_empty());

        f.clock.advance(chrono::Duration::seconds(1));
        f.host.push_tools(vec![remote_tool("search")]);
        f.service.effective_tools(&caller, &both).await.unwrap();
        assert_eq!(f.host.endpoints().len(), tries + 4, "past the back-off");
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
        assert_eq!(unreachable(&effective), [github]);

        f.clock
            .advance(chrono::Duration::seconds(TOOL_REFRESH_BACKOFF_SECS));
        f.host.push_tools(vec![remote_tool("search")]);
        let effective = f
            .service
            .effective_tools(&caller, &selection(vec![github], vec![]))
            .await
            .unwrap();
        let endpoints = f.host.endpoints();
        assert_eq!(
            endpoints.last().unwrap().url().unwrap().as_str(),
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
        assert_eq!(
            stored.url().unwrap().as_str(),
            "https://moved-1.example.net/mcp"
        );
        assert!(stored.cache().is_none(), "the old URL's list is not stored");
        assert!(!stored.listed());

        host.push_tools(vec![remote_tool("search")]);
        let effective = service
            .effective_tools(&caller, &selection(vec![server.id()], vec![]))
            .await
            .unwrap();
        assert!(effective.tools.is_empty(), "nothing from the old URL");
        assert_eq!(unreachable(&effective), [server.id()]);
        let stored = service.get(&caller, server.id()).await.unwrap().server;
        assert_eq!(
            stored.url().unwrap().as_str(),
            "https://moved-2.example.net/mcp"
        );
        assert!(stored.cache().is_none());
    }

    struct Stdio {
        service: ToolService,
        store: Arc<MemoryStore>,
        host: Arc<ScriptedToolHost>,
        cipher: Arc<FakeCipher>,
    }

    /// A service with stdio on (`enabled`), on a store whose accounts the gate reads.
    fn stdio_fixture(enabled: bool) -> Stdio {
        let clock = Arc::new(FixedClock::default());
        let store = Arc::new(MemoryStore::new(clock.clone()));
        let host = Arc::new(ScriptedToolHost::new());
        let cipher = Arc::new(FakeCipher::new());
        let service = ToolService::new(
            store.clone(),
            host.clone(),
            cipher.clone(),
            clock,
            PrivateNetwork::Admins,
        )
        .with_stdio(StdioPolicy {
            enabled,
            accounts: store.clone(),
        });
        Stdio {
            service,
            store,
            host,
            cipher,
        }
    }

    fn program(name: &str) -> NewToolServer {
        NewToolServer {
            name: name.to_owned(),
            transport: Transport::Stdio,
            stdio: Some(StdioInput {
                command: "/usr/local/bin/files-mcp".to_owned(),
                args: vec!["--root".to_owned(), "/srv/data dir".to_owned()],
                env: headers(&[("API_KEY", "sk-TOPSECRET"), ("MODE", "read")]),
                cwd: None,
            }),
            ..NewToolServer::default()
        }
    }

    /// An administrator the store knows, so the gate lets their servers start.
    fn administrator(f: &Stdio) -> Actor {
        let caller = admin();
        f.store.add_user(&caller);
        caller
    }

    fn launch(endpoint: &ToolEndpoint) -> &StdioLaunch {
        match &endpoint.target {
            Target::Stdio(launch) => launch,
            Target::Http(url) => panic!("an HTTP endpoint: {url:?}"),
        }
    }

    #[tokio::test]
    async fn an_administrator_adds_a_stdio_server_with_a_sealed_environment() {
        let f = stdio_fixture(true);
        let caller = administrator(&f);
        let server = f.service.create(&caller, program("Files")).await.unwrap();
        assert_eq!(server.transport(), Transport::Stdio);
        assert_eq!(server.url(), None);
        assert_eq!(server.auth(), AuthMode::None);
        assert!(!server.review_required(), "nothing to review yet");
        let stored = server.stdio().unwrap();
        assert_eq!(stored.command, "/usr/local/bin/files-mcp");
        assert_eq!(stored.args, ["--root", "/srv/data dir"], "kept as entered");
        assert_eq!(stored.cwd, server.id().to_string(), "its own directory");
        let sealed = stored.env_sealed.as_deref().unwrap();
        let opened = f.cipher.open(sealed, &server.env_context()).unwrap();
        assert_eq!(
            StdioEnv::entries(&opened).unwrap(),
            [
                ("API_KEY".to_owned(), "sk-TOPSECRET".to_owned()),
                ("MODE".to_owned(), "read".to_owned()),
            ]
        );
        assert_eq!(f.service.env_names(&server), ["API_KEY", "MODE"]);
        let details = f.service.get(&caller, server.id()).await.unwrap();
        assert_eq!(details.env, KeyState::Set);
        assert_eq!(details.credentials, KeyState::Missing);
        assert_eq!(details.sign_in, None);

        let named = NewToolServer {
            stdio: Some(StdioInput {
                cwd: Some("shared/files".to_owned()),
                env: BTreeMap::new(),
                ..program("Other").stdio.unwrap()
            }),
            ..program("Other")
        };
        let other = f.service.create(&caller, named).await.unwrap();
        assert_eq!(other.stdio().unwrap().cwd, "shared/files");
        assert_eq!(other.stdio().unwrap().env_sealed, None);
        let details = f.service.get(&caller, other.id()).await.unwrap();
        assert_eq!(details.env, KeyState::Missing);
    }

    #[tokio::test]
    async fn only_administrators_add_stdio_servers_and_only_while_it_is_on() {
        let f = stdio_fixture(true);
        let user = actor();
        let error = f.service.create(&user, program("Files")).await.unwrap_err();
        assert_eq!(error.code(), "FORBIDDEN");

        let off = stdio_fixture(false);
        let caller = administrator(&off);
        let error = off
            .service
            .create(&caller, program("Files"))
            .await
            .unwrap_err();
        assert_eq!(error.code(), STDIO_DISABLED);

        // Without `with_stdio` (the composition root's default) it is off too.
        let plain = fixture();
        let error = plain
            .service
            .create(&admin(), program("Files"))
            .await
            .unwrap_err();
        assert_eq!(error.code(), STDIO_DISABLED);
        assert!(!plain.service.stdio_available(&admin()));
        assert!(f.service.stdio_available(&caller));
        assert!(!f.service.stdio_available(&user));
        assert!(!off.service.stdio_available(&caller));
        assert!(f.store.list(user.user_id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_stdio_server_takes_a_program_and_nothing_else() {
        let f = stdio_fixture(true);
        let caller = administrator(&f);
        let field = |error: DomainError| match error {
            DomainError::Validation { field, .. } => field.unwrap_or_default(),
            other => panic!("not a validation error: {other:?}"),
        };
        let cases = [
            (
                NewToolServer {
                    url: "https://mcp.example.com/mcp".to_owned(),
                    ..program("A")
                },
                "url",
            ),
            (
                NewToolServer {
                    bearer: Some("tok".to_owned()),
                    ..program("A")
                },
                "authMode",
            ),
            (
                NewToolServer {
                    headers: headers(&[("X-Key", "v")]),
                    ..program("A")
                },
                "authMode",
            ),
            (
                NewToolServer {
                    auth: Some(AuthMode::OAuth),
                    ..program("A")
                },
                "authMode",
            ),
            (
                NewToolServer {
                    stdio: None,
                    ..program("A")
                },
                "stdio.command",
            ),
            (
                NewToolServer {
                    stdio: Some(StdioInput {
                        command: "npx".to_owned(),
                        ..StdioInput::default()
                    }),
                    ..program("A")
                },
                "stdio.command",
            ),
            (
                NewToolServer {
                    stdio: Some(StdioInput {
                        cwd: Some("../etc".to_owned()),
                        ..program("A").stdio.unwrap()
                    }),
                    ..program("A")
                },
                "stdio.cwd",
            ),
            (
                NewToolServer {
                    stdio: Some(StdioInput {
                        env: headers(&[("BAD NAME", "v")]),
                        ..program("A").stdio.unwrap()
                    }),
                    ..program("A")
                },
                "stdio.env",
            ),
        ];
        for (input, expected) in cases {
            let error = f.service.create(&caller, input).await.unwrap_err();
            assert_eq!(field(error), expected);
        }
        // An HTTP server takes no program.
        let http = NewToolServer {
            stdio: program("A").stdio,
            ..named("A")
        };
        assert_eq!(
            field(f.service.create(&caller, http).await.unwrap_err()),
            "stdio"
        );
        assert!(f.service.list(&caller).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn changing_the_program_needs_an_administrator_and_a_new_review() {
        let f = stdio_fixture(true);
        let caller = administrator(&f);
        let server = f.service.create(&caller, program("Files")).await.unwrap();
        f.host.push_tools(vec![remote_tool("read")]);
        let tested = f.service.test(&caller, server.id()).await.unwrap().server;
        assert!(tested.listed());
        let sealed = tested.stdio().unwrap().env_sealed.clone();

        // The same environment again is no change.
        let same = ToolServerUpdate {
            stdio: Some(StdioUpdate {
                env: Some(headers(&[("API_KEY", "sk-TOPSECRET"), ("MODE", "read")])),
                ..StdioUpdate::default()
            }),
            ..ToolServerUpdate::default()
        };
        let kept = f.service.update(&caller, server.id(), same).await.unwrap();
        assert_eq!(kept.stdio().unwrap().env_sealed, sealed);
        assert!(!kept.review_required());
        assert!(kept.listed());

        // A plain account (even the owner after a demotion) changes the name, never the program.
        let demoted = Actor::user(caller.user_id);
        let rename = ToolServerUpdate {
            name: Some("Files 2".to_owned()),
            ..ToolServerUpdate::default()
        };
        f.service
            .update(&demoted, server.id(), rename)
            .await
            .unwrap();
        let change = ToolServerUpdate {
            stdio: Some(StdioUpdate {
                args: Some(vec!["--write".to_owned()]),
                ..StdioUpdate::default()
            }),
            ..ToolServerUpdate::default()
        };
        let error = f
            .service
            .update(&demoted, server.id(), change.clone())
            .await
            .unwrap_err();
        assert_eq!(error.code(), "FORBIDDEN");

        let changed = f
            .service
            .update(&caller, server.id(), change)
            .await
            .unwrap();
        let command = changed.stdio().unwrap();
        assert_eq!(command.args, ["--write"]);
        assert_eq!(command.command, "/usr/local/bin/files-mcp", "kept");
        assert_eq!(command.env_sealed, sealed, "kept");
        assert!(changed.review_required(), "maybe another server now");
        assert!(
            !changed.listed(),
            "listed again before its tools are offered"
        );

        // A blank directory goes back to the server's own.
        let reset = ToolServerUpdate {
            stdio: Some(StdioUpdate {
                cwd: Some(" ".to_owned()),
                env: Some(BTreeMap::new()),
                ..StdioUpdate::default()
            }),
            ..ToolServerUpdate::default()
        };
        let reset = f.service.update(&caller, server.id(), reset).await.unwrap();
        assert_eq!(reset.stdio().unwrap().cwd, server.id().to_string());
        assert_eq!(reset.stdio().unwrap().env_sealed, None, "all removed");
    }

    #[tokio::test]
    async fn a_stdio_server_never_takes_a_url_or_credentials() {
        let f = stdio_fixture(true);
        let caller = administrator(&f);
        let server = f.service.create(&caller, program("Files")).await.unwrap();
        let updates = [
            ToolServerUpdate {
                url: Some("https://mcp.example.com/mcp".to_owned()),
                ..ToolServerUpdate::default()
            },
            ToolServerUpdate {
                credentials: Some(CredentialChange::Set {
                    bearer: Some("tok".to_owned()),
                    headers: BTreeMap::new(),
                }),
                ..ToolServerUpdate::default()
            },
            ToolServerUpdate {
                auth: Some(AuthMode::OAuth),
                ..ToolServerUpdate::default()
            },
            ToolServerUpdate {
                client: Some(ClientChange::Set(ClientInput {
                    id: "client".to_owned(),
                    secret: None,
                })),
                ..ToolServerUpdate::default()
            },
        ];
        for update in updates {
            let error = f
                .service
                .update(&caller, server.id(), update)
                .await
                .unwrap_err();
            assert_eq!(error.code(), "VALIDATION_ERROR");
        }
        let stored = f.service.get(&caller, server.id()).await.unwrap().server;
        assert_eq!(stored, server, "nothing changed");

        let http = f.service.create(&caller, named("Web")).await.unwrap();
        let program = ToolServerUpdate {
            stdio: Some(StdioUpdate {
                command: Some("/bin/server".to_owned()),
                ..StdioUpdate::default()
            }),
            ..ToolServerUpdate::default()
        };
        let error = f
            .service
            .update(&caller, http.id(), program)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "VALIDATION_ERROR");
    }

    #[tokio::test]
    async fn the_endpoint_carries_the_program_and_a_gate_that_checks_again() {
        let f = stdio_fixture(true);
        let caller = administrator(&f);
        let server = f.service.create(&caller, program("Files")).await.unwrap();
        let endpoint = f.service.endpoint(&caller, &server).unwrap();
        assert_eq!(endpoint.url(), None);
        assert_eq!(endpoint.headers, None);
        assert_eq!(endpoint.tokens, None);
        let launch = launch(&endpoint);
        assert_eq!(launch.command, "/usr/local/bin/files-mcp");
        assert_eq!(launch.args, ["--root", "/srv/data dir"]);
        assert_eq!(launch.cwd, server.id().to_string());
        let env = StdioEnv::entries(launch.env.as_ref().unwrap()).unwrap();
        assert_eq!(env[0], ("API_KEY".to_owned(), "sk-TOPSECRET".to_owned()));
        assert!(!format!("{endpoint:?}").contains("TOPSECRET"));
        launch.gate.0.allow().await.expect("an administrator");

        // Demoted after the run started: the next start is refused.
        f.store.add_user(&Actor::user(caller.user_id));
        let error = launch.gate.0.allow().await.unwrap_err();
        assert_eq!(error.code(), "FORBIDDEN");
        f.store.add_user(&caller);
        f.store.disable_user(caller.user_id);
        assert_eq!(launch.gate.0.allow().await.unwrap_err().code(), "FORBIDDEN");

        // The session's actor counts too, before any start.
        let error = f
            .service
            .endpoint(&Actor::user(caller.user_id), &server)
            .unwrap_err();
        assert_eq!(error.code(), "FORBIDDEN");
        let off = stdio_fixture(false);
        let error = off.service.endpoint(&caller, &server).unwrap_err();
        assert_eq!(error.code(), STDIO_DISABLED);
    }

    #[tokio::test]
    async fn turns_list_and_offer_stdio_servers_only_to_administrators() {
        let f = stdio_fixture(true);
        let caller = administrator(&f);
        let server = f.service.create(&caller, program("Files")).await.unwrap();
        f.host.push_tools(vec![remote_tool("read")]);
        let effective = f
            .service
            .effective_tools(&caller, &selection(vec![server.id()], vec![]))
            .await
            .unwrap();
        assert_eq!(aliases(&effective), ["files__read"]);
        let listed = f.host.endpoints();
        assert_eq!(listed.len(), 1);
        assert_eq!(launch(&listed[0]).command, "/usr/local/bin/files-mcp");

        let demoted = Actor::user(caller.user_id);
        let effective = f
            .service
            .effective_tools(&demoted, &selection(vec![server.id()], vec![]))
            .await
            .unwrap();
        assert!(effective.tools.is_empty(), "the cached list is not offered");
        assert_eq!(unreachable(&effective), [server.id()]);
        assert_eq!(f.host.endpoints().len(), 1, "never listed for them");
        let error = f.service.test(&demoted, server.id()).await.unwrap_err();
        assert_eq!(error.code(), "FORBIDDEN");
        assert_eq!(f.host.endpoints().len(), 1);
    }
}
