//! OAuth sign-in to tool servers (`docs/design/copilot.md`, Tool Server Sign-In): starting a
//! sign-in, its callback, signing out, and the access tokens a run sends, refreshed when they
//! expire or the server answers `401`.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use futures_util::lock::Mutex as AsyncMutex;
use url::{Host, Url};
use uuid::Uuid;

use crate::domains::tool::auth::{
    AuthMode, Pending, Secrets, Session, SignIn, SignInFlow, SignInState, challenge, state_hash,
};
use crate::domains::tool::entities::ToolServer;
use crate::domains::tool::ports::SignInRepository;
use crate::domains::tool::service::ToolService;
use crate::ports::tool_auth::{
    AccessTokens, ClientAuth, Discovery, Grant, OAUTH_REJECTED, OAuthClient, Registration,
    SIGN_IN_REQUIRED, TokenRequest, TokenSet, TokenSource, ToolAuth,
};
use crate::shared::actor::Actor;
use crate::shared::constants::SIGN_IN_FLOW_SECS;
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::ToolServerId;
use crate::shared::net::NetworkScope;
use crate::shared::secret::Secret;

/// The error code when this install cannot run sign-ins (`PUBLIC_URL` is unset).
pub const SIGN_IN_UNAVAILABLE: &str = "SIGN_IN_UNAVAILABLE";
/// The error code when the authorization server offers no way to get a client id: neither Client
/// ID Metadata Documents (with an `https` `PUBLIC_URL`) nor dynamic registration.
pub const CLIENT_ID_REQUIRED: &str = "CLIENT_ID_REQUIRED";

/// The client name sent with dynamic registration (and in the Client ID Metadata Document).
pub const CLIENT_NAME: &str = "Copilot";

/// Where sign-ins come back to, from `PUBLIC_URL`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignInConfig {
    /// `<PUBLIC_URL>/api/oauth/callback`, the redirect URI of every authorization request.
    pub redirect_uri: Url,
    /// `<PUBLIC_URL>/api/oauth/client`, the Client ID Metadata Document, when `PUBLIC_URL` is
    /// `https` and `OAUTH_CLIENT_METADATA` allows it; used as the client id by authorization
    /// servers that support such documents.
    pub client_metadata: Option<Url>,
}

/// What the OAuth parts of [`ToolService`] share.
pub(crate) struct OAuth {
    auth: Arc<dyn ToolAuth>,
    flows: Arc<dyn SignInRepository>,
    config: SignInConfig,
    /// One refresh per server at a time in this process (a rotated refresh token works once).
    locks: Mutex<HashMap<ToolServerId, Arc<AsyncMutex<()>>>>,
}

impl OAuth {
    fn lock(&self, id: ToolServerId) -> Arc<AsyncMutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
        locks.entry(id).or_default().clone()
    }
}

/// A started sign-in: the URL to open in the user's browser, valid until `expires_at`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignInStart {
    pub url: Url,
    pub expires_at: DateTime<Utc>,
}

/// The query of a redirect to the callback. `Debug` hides the code and the state.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct Callback {
    pub state: String,
    pub code: Option<String>,
    /// RFC 9207 `iss`.
    pub iss: Option<String>,
    /// The authorization server's `error` (never shown: it is untrusted text).
    pub error: Option<String>,
}

impl fmt::Debug for Callback {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Callback")
            .field("iss", &self.iss)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

/// How a callback ended, for the page the browser is sent to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SignInOutcome {
    /// The server the sign-in was for, when the `state` named a live flow.
    pub server_id: Option<ToolServerId>,
    pub result: SignInResult,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignInResult {
    SignedIn,
    /// The user declined at the authorization server.
    Denied,
    /// The `state` is unknown, was used before, or its sign-in took too long.
    Expired,
    /// Anything else: an issuer mismatch, a refused code, a server that changed meanwhile.
    Failed,
}

impl SignInResult {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SignedIn => "signed-in",
            Self::Denied => "denied",
            Self::Expired => "expired",
            Self::Failed => "failed",
        }
    }
}

impl ToolService {
    /// Enables OAuth sign-in: `auth` makes the OAuth requests, `flows` keeps sign-ins in
    /// progress, and `config` says where they come back to. Without it, `OAuth` servers fail
    /// with `SIGN_IN_UNAVAILABLE`.
    #[must_use]
    pub fn with_sign_in(
        mut self,
        auth: Arc<dyn ToolAuth>,
        flows: Arc<dyn SignInRepository>,
        config: SignInConfig,
    ) -> Self {
        self.oauth = Some(Arc::new(OAuth {
            auth,
            flows,
            config,
            locks: Mutex::default(),
        }));
        self
    }

    /// The sign-in settings, when sign-in is enabled.
    pub fn sign_in_config(&self) -> Option<&SignInConfig> {
        self.oauth.as_ref().map(|oauth| &oauth.config)
    }

    /// Starts signing in to an `OAuth` server: discovers its authorization server, gets a client
    /// id (the pre-registered one, else this install's Client ID Metadata Document when the
    /// authorization server supports those, else dynamic registration), and stores the PKCE
    /// verifier and `state` for `SIGN_IN_FLOW_SECS`, replacing an earlier sign-in of the server
    /// that never came back. Returns the URL to open.
    pub async fn sign_in(&self, actor: &Actor, id: ToolServerId) -> DomainResult<SignInStart> {
        let oauth = self.oauth()?;
        let server = self.load(actor, id).await?;
        if server.auth() != AuthMode::OAuth {
            return Err(DomainError::validation(
                "Choose OAuth sign-in as this server's authentication first.",
            )
            .with_field("authMode"));
        }
        let network = self.network.scope(actor);
        let discovery = oauth.auth.discover(server.url(), network).await?;
        let scope = scope(&discovery);
        let client = self
            .oauth_client(oauth, &server, &discovery, scope.clone(), network)
            .await?;
        let secrets = Secrets::generate()?;
        let state = secrets.state.text().unwrap_or_default().to_owned();
        let now = self.clock.now();
        let pending = Pending {
            verifier: secrets.verifier,
            redirect_uri: oauth.config.redirect_uri.clone(),
            issuer: discovery.server.issuer.clone(),
            iss_parameter: discovery.server.iss_parameter,
            token_endpoint: discovery.server.token_endpoint.clone(),
            client,
            resource: discovery.resource.clone(),
            scope: scope.clone(),
            url: server.url().as_str().to_owned(),
            private: network == NetworkScope::Private,
        };
        let mut flow = SignInFlow {
            id: Uuid::now_v7(),
            user_id: actor.user_id,
            server_id: id,
            state_hash: state_hash(&state),
            sealed: Vec::new(),
            created_at: now,
            expires_at: now + Duration::seconds(SIGN_IN_FLOW_SECS),
        };
        flow.sealed = self.cipher.seal(&pending.to_json(), &flow.context())?;
        oauth.flows.insert(&flow).await?;

        let mut url = discovery.server.authorization_endpoint.clone();
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("response_type", "code")
                .append_pair("client_id", &pending.client.id)
                .append_pair("redirect_uri", pending.redirect_uri.as_str())
                .append_pair("code_challenge", &challenge(&pending.verifier))
                .append_pair("code_challenge_method", "S256")
                .append_pair("state", &state)
                .append_pair("resource", &pending.resource);
            if let Some(scope) = &scope {
                query.append_pair("scope", scope);
            }
        }
        tracing::info!(server_id = %id, user_id = %actor.user_id, "core.tools.sign-in.started");
        Ok(SignInStart {
            url,
            expires_at: flow.expires_at,
        })
    }

    /// Finishes a sign-in from the authorization server's redirect. The `state` must name a
    /// stored flow, which this uses up whatever happens next; the flow must still be live; a
    /// present `iss` must equal the recorded issuer, and an absent one is refused when the
    /// issuer advertises it (RFC 9207); the server must still be the user's, in `OAuth` mode,
    /// at the same URL. Then the code is exchanged and the tokens are stored sealed for the
    /// flow's user and server only. Never fails: the outcome says what happened, and the
    /// authorization server's error text is never passed on.
    pub async fn complete(&self, callback: &Callback) -> SignInOutcome {
        match self.finish(callback).await {
            Ok(outcome) | Err(outcome) => outcome,
        }
    }

    /// [`ToolService::complete`], with every early end as `Err`.
    async fn finish(&self, callback: &Callback) -> Result<SignInOutcome, SignInOutcome> {
        let oauth = self
            .oauth
            .as_ref()
            .ok_or_else(|| failed(None, "unavailable"))?;
        let flow = self.live(oauth, callback).await?;
        let server_id = Some(flow.server_id);
        let pending = self
            .cipher
            .open(&flow.sealed, &flow.context())
            .and_then(|secret| Pending::parse(&secret))
            .map_err(|_| failed(server_id, "unreadable"))?;
        let code = accepted(callback, &pending, flow.server_id)?;
        let server = match self.servers.get(flow.user_id, flow.server_id).await {
            Ok(Some(server))
                if server.auth() == AuthMode::OAuth && server.url().as_str() == pending.url =>
            {
                server
            }
            Ok(_) => return Err(failed(server_id, "server-changed")),
            Err(_) => return Err(failed(server_id, "storage")),
        };
        let tokens = exchange(oauth, &pending, code).await.map_err(|error| {
            tracing::warn!(server_id = %flow.server_id, code = error.code(), "core.tools.sign-in.exchange-failed");
            failed(server_id, "exchange")
        })?;
        self.store_sign_in(&flow, &server, pending, &tokens).await?;
        // A new sign-in may fix what failed: the next turn lists the server again.
        self.failures().remove(&server.id());
        tracing::info!(server_id = %server.id(), user_id = %flow.user_id, "core.tools.sign-in.completed");
        Ok(SignInOutcome {
            server_id,
            result: SignInResult::SignedIn,
        })
    }

    /// Takes the flow the callback's `state` names, which uses it up; it must still be live.
    async fn live(&self, oauth: &OAuth, callback: &Callback) -> Result<SignInFlow, SignInOutcome> {
        let flow = match oauth.flows.take(&state_hash(&callback.state)).await {
            Ok(Some(flow)) => flow,
            Ok(None) => {
                tracing::warn!("core.tools.sign-in.unknown-state");
                return Err(SignInOutcome {
                    server_id: None,
                    result: SignInResult::Expired,
                });
            }
            Err(error) => {
                tracing::warn!(error = %error, "core.tools.sign-in.flow-failed");
                return Err(failed(None, "storage"));
            }
        };
        if flow.expires_at <= self.clock.now() {
            tracing::info!(server_id = %flow.server_id, "core.tools.sign-in.expired");
            return Err(SignInOutcome {
                server_id: Some(flow.server_id),
                result: SignInResult::Expired,
            });
        }
        Ok(flow)
    }

    /// Seals the exchanged tokens for the flow's server and stores them, if the server still has
    /// the URL the sign-in started with.
    async fn store_sign_in(
        &self,
        flow: &SignInFlow,
        server: &ToolServer,
        pending: Pending,
        tokens: &TokenSet,
    ) -> Result<(), SignInOutcome> {
        let server_id = Some(flow.server_id);
        let session = Session {
            issuer: pending.issuer,
            token_endpoint: pending.token_endpoint,
            client: pending.client,
            resource: pending.resource,
            scope: pending.scope,
            access: Secret::new(""),
            refresh: None,
        }
        .renewed(tokens.clone());
        let sign_in = self
            .seal_sign_in(server, &session, tokens)
            .map_err(|_| failed(server_id, "seal"))?;
        match self
            .servers
            .save_sign_in(
                flow.user_id,
                server.id(),
                server.url(),
                Some(&sign_in),
                None,
            )
            .await
        {
            Ok(true) => Ok(()),
            Ok(false) => Err(failed(server_id, "server-changed")),
            Err(_) => Err(failed(server_id, "storage")),
        }
    }

    /// Forgets the server's sign-in (the authorization server keeps its grant until it
    /// expires; revoke it there to end it everywhere).
    pub async fn sign_out(&self, actor: &Actor, id: ToolServerId) -> DomainResult<ToolServer> {
        let server = self.load(actor, id).await?;
        self.servers
            .save_sign_in(actor.user_id, id, server.url(), None, None)
            .await?;
        tracing::info!(server_id = %id, "core.tools.sign-in.signed-out");
        self.load(actor, id).await
    }

    /// The access tokens of an `OAuth` server for its endpoint. Fails with `SIGN_IN_REQUIRED`
    /// when it has no sign-in or the sign-in expired without a refresh token.
    pub(crate) fn tokens(&self, actor: &Actor, server: &ToolServer) -> DomainResult<TokenSource> {
        self.oauth()?;
        match server.sign_in_state(self.clock.now()) {
            Some(SignInState::SignedIn) => Ok(TokenSource(Arc::new(ServerTokens {
                service: self.clone(),
                actor: *actor,
                server: server.clone(),
            }))),
            _ => Err(sign_in_required(server)),
        }
    }

    fn oauth(&self) -> DomainResult<&Arc<OAuth>> {
        self.oauth.as_ref().ok_or_else(|| {
            DomainError::business_rule(
                "Sign-in to tool servers is not set up on this Copilot server: its administrator needs to set PUBLIC_URL.",
                SIGN_IN_UNAVAILABLE,
            )
        })
    }

    /// The client id of a sign-in, in the order the MCP specification gives.
    async fn oauth_client(
        &self,
        oauth: &OAuth,
        server: &ToolServer,
        discovery: &Discovery,
        scope: Option<String>,
        network: NetworkScope,
    ) -> DomainResult<OAuthClient> {
        let methods = &discovery.server.auth_methods;
        if let Some(config) = server.client() {
            let secret = config
                .secret_sealed
                .as_deref()
                .map(|sealed| self.cipher.open(sealed, &server.client_context()))
                .transpose()?;
            let auth = match &secret {
                None => ClientAuth::None,
                Some(_)
                    if !methods.is_empty()
                        && !methods.iter().any(|method| method == "client_secret_basic")
                        && methods.iter().any(|method| method == "client_secret_post") =>
                {
                    ClientAuth::Post
                }
                Some(_) => ClientAuth::Basic,
            };
            return Ok(OAuthClient {
                id: config.id.clone(),
                secret,
                auth,
            });
        }
        if discovery.server.client_metadata
            && let Some(document) = &oauth.config.client_metadata
        {
            return Ok(OAuthClient {
                id: document.as_str().to_owned(),
                secret: None,
                auth: ClientAuth::None,
            });
        }
        if discovery.server.registration_endpoint.is_some() {
            let registration = Registration {
                client_name: CLIENT_NAME.to_owned(),
                redirect_uri: oauth.config.redirect_uri.clone(),
                application_type: if loopback(&oauth.config.redirect_uri) {
                    "native"
                } else {
                    "web"
                },
                scope,
            };
            return oauth
                .auth
                .register(&discovery.server, &registration, network)
                .await;
        }
        Err(DomainError::business_rule(
            "This tool server's sign-in needs a client ID: register Copilot with its authorization server, then enter the client ID in the server's settings.",
            CLIENT_ID_REQUIRED,
        ))
    }

    /// Seals `session` for `server`'s row, with when its access token expires.
    fn seal_sign_in(
        &self,
        server: &ToolServer,
        session: &Session,
        tokens: &TokenSet,
    ) -> DomainResult<SignIn> {
        let expires_at = tokens
            .expires_in
            .and_then(|lifetime| Duration::from_std(lifetime).ok())
            .and_then(|lifetime| self.clock.now().checked_add_signed(lifetime));
        Ok(SignIn {
            sealed: self
                .cipher
                .seal(&session.to_json(), &server.sign_in_context())?,
            expires_at,
            refreshable: session.refresh.is_some(),
        })
    }
}

/// The code of a callback whose `iss` matches the recorded issuer (RFC 9207: a present `iss`
/// must match exactly, an absent one is refused when the issuer advertises it) and that carries
/// no error. The authorization server's error text is never used.
fn accepted<'a>(
    callback: &'a Callback,
    pending: &Pending,
    server: ToolServerId,
) -> Result<&'a str, SignInOutcome> {
    match &callback.iss {
        Some(iss) if *iss != pending.issuer => return Err(failed(Some(server), "issuer-mismatch")),
        None if pending.iss_parameter => return Err(failed(Some(server), "issuer-missing")),
        _ => {}
    }
    match &callback.error {
        Some(error) if error == "access_denied" => {
            tracing::info!(server_id = %server, "core.tools.sign-in.denied");
            Err(SignInOutcome {
                server_id: Some(server),
                result: SignInResult::Denied,
            })
        }
        Some(_) => Err(failed(Some(server), "authorization-error")),
        None => callback
            .code
            .as_deref()
            .filter(|code| !code.is_empty())
            .ok_or_else(|| failed(Some(server), "no-code")),
    }
}

/// Exchanges the code for tokens at the token endpoint the sign-in recorded.
async fn exchange(oauth: &OAuth, pending: &Pending, code: &str) -> DomainResult<TokenSet> {
    let network = if pending.private {
        NetworkScope::Private
    } else {
        NetworkScope::Public
    };
    let request = TokenRequest {
        endpoint: pending.token_endpoint.clone(),
        client: pending.client.clone(),
        resource: pending.resource.clone(),
        grant: Grant::Code {
            code: Secret::new(code),
            verifier: pending.verifier.clone(),
            redirect_uri: pending.redirect_uri.clone(),
        },
    };
    oauth.auth.token(&request, network).await
}

/// A callback that went wrong, logged with a short reason.
fn failed(server_id: Option<ToolServerId>, reason: &'static str) -> SignInOutcome {
    let server = server_id.map_or_else(String::new, |id| id.to_string());
    tracing::warn!(server_id = %server, reason, "core.tools.sign-in.failed");
    SignInOutcome {
        server_id,
        result: SignInResult::Failed,
    }
}

/// The scopes to ask for: what discovery chose, plus `offline_access` (a refresh token) when the
/// authorization server lists it. `None` sends no `scope`.
fn scope(discovery: &Discovery) -> Option<String> {
    let mut scopes = discovery.scopes.clone();
    if discovery.server.offline_access && !scopes.iter().any(|scope| scope == "offline_access") {
        scopes.push("offline_access".to_owned());
    }
    (!scopes.is_empty()).then(|| scopes.join(" "))
}

fn loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(domain)) => domain == "localhost",
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

fn sign_in_required(server: &ToolServer) -> DomainError {
    DomainError::business_rule(
        format!(
            "Sign in to the tool server {} again in its settings.",
            server.name()
        ),
        SIGN_IN_REQUIRED,
    )
}

/// The access tokens of one `OAuth` server during a run or a listing, on the sign-in the run
/// started with; a refresh reads and writes the stored one.
struct ServerTokens {
    service: ToolService,
    actor: Actor,
    server: ToolServer,
}

#[async_trait]
impl AccessTokens for ServerTokens {
    async fn current(&self) -> DomainResult<Secret> {
        let now = self.service.clock.now();
        match self.server.sign_in() {
            Some(sign_in) if !sign_in.expired(now) => Ok(self.open(&self.server, sign_in)?.access),
            _ => self.renew(None).await,
        }
    }

    async fn rejected(&self, token: &Secret) -> DomainResult<Secret> {
        self.renew(Some(token)).await
    }
}

impl ServerTokens {
    fn open(&self, server: &ToolServer, sign_in: &SignIn) -> DomainResult<Session> {
        let secret = self
            .service
            .cipher
            .open(&sign_in.sealed, &server.sign_in_context())?;
        Session::parse(&secret)
    }

    /// The stored sign-in's access token, refreshed when it expired or is the `rejected` one.
    /// Refreshes of one server run one at a time, and each first reads what the last one stored.
    async fn renew(&self, rejected: Option<&Secret>) -> DomainResult<Secret> {
        let service = &self.service;
        let oauth = service.oauth()?;
        let lock = oauth.lock(self.server.id());
        let _guard = lock.lock().await;
        let required = || sign_in_required(&self.server);
        let server = service
            .servers
            .get(self.actor.user_id, self.server.id())
            .await?
            .filter(|server| server.auth() == AuthMode::OAuth && server.url() == self.server.url())
            .ok_or_else(required)?;
        let sign_in = server.sign_in().cloned().ok_or_else(required)?;
        let session = self.open(&server, &sign_in)?;
        match rejected {
            // Another request already replaced the token the server rejected.
            Some(token) if session.access != *token => return Ok(session.access),
            None if !sign_in.expired(service.clock.now()) => return Ok(session.access),
            _ => {}
        }
        if session.refresh.is_none() {
            if rejected.is_some() {
                self.clear(&server, &sign_in).await?;
            }
            return Err(required());
        }
        self.refresh(oauth, &server, &sign_in, session).await
    }

    /// Refreshes `session` (which has a refresh token) and stores the result in place of
    /// `sign_in`. A rejected refresh token ends the sign-in; any other failure keeps it.
    async fn refresh(
        &self,
        oauth: &OAuth,
        server: &ToolServer,
        sign_in: &SignIn,
        session: Session,
    ) -> DomainResult<Secret> {
        let service = &self.service;
        let id = server.id();
        let request = TokenRequest {
            endpoint: session.token_endpoint.clone(),
            client: session.client.clone(),
            resource: session.resource.clone(),
            grant: Grant::Refresh {
                token: session.refresh.clone().unwrap_or_else(|| Secret::new("")),
            },
        };
        let network = service.network.scope(&self.actor);
        let tokens = match oauth.auth.token(&request, network).await {
            Ok(tokens) => tokens,
            Err(error) if error.code() == OAUTH_REJECTED => {
                tracing::warn!(server_id = %id, "core.tools.sign-in.refresh-rejected");
                self.clear(server, sign_in).await?;
                return Err(sign_in_required(server));
            }
            Err(error) => {
                tracing::warn!(server_id = %id, code = error.code(), "core.tools.sign-in.refresh-failed");
                return Err(error);
            }
        };
        let renewed = session.renewed(tokens.clone());
        let next = service.seal_sign_in(server, &renewed, &tokens)?;
        let saved = service
            .servers
            .save_sign_in(
                self.actor.user_id,
                id,
                server.url(),
                Some(&next),
                Some(&sign_in.sealed),
            )
            .await?;
        if !saved {
            // Signed out or signed in again meanwhile: that sign-in decides.
            return Err(sign_in_required(server));
        }
        tracing::info!(server_id = %id, "core.tools.sign-in.refreshed");
        Ok(renewed.access)
    }

    /// The sign-in is no good any more: the server needs a new one.
    async fn clear(&self, server: &ToolServer, sign_in: &SignIn) -> DomainResult<()> {
        self.service
            .servers
            .save_sign_in(
                self.actor.user_id,
                server.id(),
                server.url(),
                None,
                Some(&sign_in.sealed),
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::domains::tool::service::{
        ClientChange, ClientInput, CredentialChange, NewToolServer, ToolServerUpdate,
    };
    use crate::ports::clock::Clock;
    use crate::ports::secret_cipher::SecretCipher;
    use crate::ports::tool_auth::{OAUTH_FAILED, SIGN_IN_REQUIRED};
    use crate::shared::net::PrivateNetwork;
    use crate::testing::{
        FakeCipher, FixedClock, MemoryStore, ScriptedAuth, ScriptedToolHost, actor, tokens,
    };

    const REDIRECT: &str = "https://copilot.example.com/api/oauth/callback";

    struct Fixture {
        service: ToolService,
        store: Arc<MemoryStore>,
        auth: Arc<ScriptedAuth>,
        cipher: Arc<FakeCipher>,
        clock: Arc<FixedClock>,
    }

    fn fixture() -> Fixture {
        let clock = Arc::new(FixedClock::default());
        let store = Arc::new(MemoryStore::new(clock.clone()));
        let cipher = Arc::new(FakeCipher::new());
        let auth = Arc::new(ScriptedAuth::new());
        let service = ToolService::new(
            store.clone(),
            Arc::new(ScriptedToolHost::new()),
            cipher.clone(),
            clock.clone(),
            PrivateNetwork::Admins,
        )
        .with_sign_in(
            auth.clone(),
            store.clone(),
            SignInConfig {
                redirect_uri: Url::parse(REDIRECT).unwrap(),
                client_metadata: Some(
                    Url::parse("https://copilot.example.com/api/oauth/client").unwrap(),
                ),
            },
        );
        Fixture {
            service,
            store,
            auth,
            cipher,
            clock,
        }
    }

    fn oauth_server() -> NewToolServer {
        NewToolServer {
            name: "Docs".to_owned(),
            url: "https://mcp.example.com/mcp".to_owned(),
            auth: Some(AuthMode::OAuth),
            ..NewToolServer::default()
        }
    }

    fn field(error: &DomainError) -> Option<&str> {
        match error {
            DomainError::Validation { field, .. } => field.as_deref(),
            _ => None,
        }
    }

    fn query(url: &Url) -> BTreeMap<String, String> {
        url.query_pairs().into_owned().collect()
    }

    /// Starts a sign-in and returns its `state`.
    async fn start(f: &Fixture, actor: &Actor, id: ToolServerId) -> String {
        let started = f.service.sign_in(actor, id).await.unwrap();
        query(&started.url)["state"].clone()
    }

    fn callback(state: &str) -> Callback {
        Callback {
            state: state.to_owned(),
            code: Some("code-1".to_owned()),
            iss: Some("https://auth.example.com".to_owned()),
            error: None,
        }
    }

    async fn signed_in(f: &Fixture, actor: &Actor) -> ToolServer {
        let server = f.service.create(actor, oauth_server()).await.unwrap();
        let state = start(f, actor, server.id()).await;
        f.auth.answer(Ok(tokens("access-1", Some("refresh-1"))));
        let outcome = f.service.complete(&callback(&state)).await;
        assert_eq!(outcome.result, SignInResult::SignedIn);
        f.service.get(actor, server.id()).await.unwrap().server
    }

    fn session(f: &Fixture, server: &ToolServer) -> Session {
        let sign_in = server.sign_in().expect("a sign-in");
        let secret = f
            .cipher
            .open(&sign_in.sealed, &server.sign_in_context())
            .unwrap();
        Session::parse(&secret).unwrap()
    }

    #[tokio::test]
    async fn needs_public_url_and_oauth_mode() {
        let f = fixture();
        let user = actor();
        let plain = f
            .service
            .create(
                &user,
                NewToolServer {
                    auth: None,
                    ..oauth_server()
                },
            )
            .await
            .unwrap();
        let error = f.service.sign_in(&user, plain.id()).await.unwrap_err();
        assert_eq!(error.code(), "VALIDATION_ERROR");

        let without = ToolService::new(
            f.store.clone(),
            Arc::new(ScriptedToolHost::new()),
            f.cipher.clone(),
            f.clock.clone(),
            PrivateNetwork::Admins,
        );
        let server = without.create(&user, oauth_server()).await.unwrap();
        let error = without.sign_in(&user, server.id()).await.unwrap_err();
        assert_eq!(error.code(), SIGN_IN_UNAVAILABLE);
        let error = without.endpoint(&user, &server).unwrap_err();
        assert_eq!(error.code(), SIGN_IN_UNAVAILABLE);
    }

    #[tokio::test]
    async fn builds_an_authorization_url_with_pkce_state_and_resource() {
        let f = fixture();
        let user = actor();
        let server = f.service.create(&user, oauth_server()).await.unwrap();
        let started = f.service.sign_in(&user, server.id()).await.unwrap();
        assert_eq!(started.url.path(), "/authorize");
        let query = query(&started.url);
        assert_eq!(query["response_type"], "code");
        assert_eq!(query["client_id"], "registered-1", "dynamic registration");
        assert_eq!(query["redirect_uri"], REDIRECT);
        assert_eq!(query["code_challenge_method"], "S256");
        assert_eq!(query["code_challenge"].len(), 43);
        assert_eq!(query["state"].len(), 43);
        assert_eq!(query["resource"], "https://mcp.example.com/mcp");
        assert_eq!(query["scope"], "files:read offline_access");
        assert_eq!(
            started.expires_at,
            f.clock.now() + Duration::seconds(SIGN_IN_FLOW_SECS)
        );
        let registration = &f.auth.registrations()[0];
        assert_eq!(registration.client_name, CLIENT_NAME);
        assert_eq!(registration.application_type, "web");
        assert_eq!(registration.redirect_uri.as_str(), REDIRECT);

        // The flow keeps only the state's hash.
        let flows = f.store.tables().sign_in_flows.clone();
        let flow = flows.values().next().unwrap();
        assert_eq!(flow.state_hash, state_hash(&query["state"]));
        assert!(
            !String::from_utf8_lossy(&flow.sealed).contains(&query["state"]),
            "the state is not stored"
        );
    }

    #[tokio::test]
    async fn picks_the_configured_client_then_the_metadata_document_then_registration() {
        let f = fixture();
        let user = actor();
        let configured = f
            .service
            .create(
                &user,
                NewToolServer {
                    client: Some(ClientInput {
                        id: "my-client".to_owned(),
                        secret: Some("my-secret".to_owned()),
                    }),
                    ..oauth_server()
                },
            )
            .await
            .unwrap();
        f.auth
            .edit(|discovery| discovery.server.client_metadata = true);
        let started = f.service.sign_in(&user, configured.id()).await.unwrap();
        assert_eq!(query(&started.url)["client_id"], "my-client");

        let other = f
            .service
            .create(
                &user,
                NewToolServer {
                    name: "Other".to_owned(),
                    ..oauth_server()
                },
            )
            .await
            .unwrap();
        let started = f.service.sign_in(&user, other.id()).await.unwrap();
        assert_eq!(
            query(&started.url)["client_id"],
            "https://copilot.example.com/api/oauth/client"
        );
        assert!(f.auth.registrations().is_empty());

        f.auth.edit(|discovery| {
            discovery.server.client_metadata = false;
            discovery.server.registration_endpoint = None;
        });
        let error = f.service.sign_in(&user, other.id()).await.unwrap_err();
        assert_eq!(error.code(), CLIENT_ID_REQUIRED);
    }

    #[tokio::test]
    async fn a_configured_secret_authenticates_the_way_the_server_takes() {
        let f = fixture();
        let user = actor();
        let server = f
            .service
            .create(
                &user,
                NewToolServer {
                    client: Some(ClientInput {
                        id: "my-client".to_owned(),
                        secret: Some("my-secret".to_owned()),
                    }),
                    ..oauth_server()
                },
            )
            .await
            .unwrap();
        for (methods, expected) in [
            (Vec::new(), ClientAuth::Basic),
            (
                vec!["client_secret_basic", "client_secret_post"],
                ClientAuth::Basic,
            ),
            (vec!["none", "client_secret_post"], ClientAuth::Post),
        ] {
            f.auth.edit(|discovery| {
                discovery.server.auth_methods = methods.iter().map(|m| (*m).to_owned()).collect();
            });
            let state = start(&f, &user, server.id()).await;
            f.auth.answer(Ok(tokens("access-1", None)));
            f.service.complete(&callback(&state)).await;
            let request = f.auth.requests().last().cloned().unwrap();
            assert_eq!(request.client.auth, expected, "{methods:?}");
            assert_eq!(request.client.secret, Some(Secret::new("my-secret")));
        }
    }

    #[tokio::test]
    async fn a_callback_stores_sealed_tokens_for_its_server_once() {
        let f = fixture();
        let user = actor();
        let server = f.service.create(&user, oauth_server()).await.unwrap();
        let state = start(&f, &user, server.id()).await;
        f.auth.answer(Ok(tokens("access-1", Some("refresh-1"))));
        let outcome = f.service.complete(&callback(&state)).await;
        assert_eq!(
            outcome,
            SignInOutcome {
                server_id: Some(server.id()),
                result: SignInResult::SignedIn,
            }
        );
        let request = &f.auth.requests()[0];
        let Grant::Code {
            code,
            verifier,
            redirect_uri,
        } = &request.grant
        else {
            panic!("a code grant");
        };
        assert_eq!(code, &Secret::new("code-1"));
        assert_eq!(redirect_uri.as_str(), REDIRECT);
        assert_eq!(request.resource, "https://mcp.example.com/mcp");
        assert_eq!(request.client.id, "registered-1");
        assert_eq!(verifier.text().unwrap().len(), 43);

        let details = f.service.get(&user, server.id()).await.unwrap();
        assert_eq!(details.sign_in, Some(SignInState::SignedIn));
        let stored = details.server;
        let sign_in = stored.sign_in().unwrap();
        assert!(sign_in.refreshable);
        assert_eq!(sign_in.expires_at, Some(f.clock.now() + Duration::hours(1)));
        let session = session(&f, &stored);
        assert_eq!(session.access, Secret::new("access-1"));
        assert_eq!(session.scope.as_deref(), Some("files:read offline_access"));
        assert!(
            f.cipher
                .open(&sign_in.sealed, &stored.headers_context())
                .is_err(),
            "bound to its column"
        );

        // The same state again: used up.
        let replay = f.service.complete(&callback(&state)).await;
        assert_eq!(replay.result, SignInResult::Expired);
        assert_eq!(replay.server_id, None);
        assert_eq!(f.auth.requests().len(), 1, "no second exchange");
    }

    #[tokio::test]
    async fn refuses_tampered_late_and_mixed_up_callbacks() {
        let f = fixture();
        let user = actor();
        let server = f.service.create(&user, oauth_server()).await.unwrap();

        let state = start(&f, &user, server.id()).await;
        let mut tampered = callback(&state);
        tampered.state.push('x');
        assert_eq!(
            f.service.complete(&tampered).await.result,
            SignInResult::Expired
        );

        let state = start(&f, &user, server.id()).await;
        let mut mixed_up = callback(&state);
        mixed_up.iss = Some("https://evil.example.com".to_owned());
        assert_eq!(
            f.service.complete(&mixed_up).await.result,
            SignInResult::Failed
        );

        let state = start(&f, &user, server.id()).await;
        let mut missing = callback(&state);
        missing.iss = None;
        assert_eq!(
            f.service.complete(&missing).await.result,
            SignInResult::Failed,
            "the issuer advertises iss"
        );

        let state = start(&f, &user, server.id()).await;
        let mut denied = callback(&state);
        denied.code = None;
        denied.error = Some("access_denied".to_owned());
        assert_eq!(
            f.service.complete(&denied).await.result,
            SignInResult::Denied
        );

        let state = start(&f, &user, server.id()).await;
        f.clock.advance(Duration::seconds(SIGN_IN_FLOW_SECS));
        assert_eq!(
            f.service.complete(&callback(&state)).await.result,
            SignInResult::Expired
        );
        assert!(f.auth.requests().is_empty(), "no code was exchanged");
        assert!(
            f.service
                .get(&user, server.id())
                .await
                .unwrap()
                .server
                .sign_in()
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_new_sign_in_replaces_one_that_never_came_back() {
        let f = fixture();
        let user = actor();
        let server = f.service.create(&user, oauth_server()).await.unwrap();
        let first = start(&f, &user, server.id()).await;
        let second = start(&f, &user, server.id()).await;
        assert_eq!(
            f.service.complete(&callback(&first)).await.result,
            SignInResult::Expired
        );
        f.auth.answer(Ok(tokens("access-1", None)));
        assert_eq!(
            f.service.complete(&callback(&second)).await.result,
            SignInResult::SignedIn
        );
    }

    #[tokio::test]
    async fn refuses_a_callback_after_the_url_or_mode_changed() {
        let f = fixture();
        let user = actor();
        let server = f.service.create(&user, oauth_server()).await.unwrap();
        let state = start(&f, &user, server.id()).await;
        f.service
            .update(
                &user,
                server.id(),
                ToolServerUpdate {
                    url: Some("https://other.example.com/mcp".to_owned()),
                    ..ToolServerUpdate::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            f.service.complete(&callback(&state)).await.result,
            SignInResult::Failed
        );
        assert!(f.auth.requests().is_empty());
    }

    #[tokio::test]
    async fn a_callback_signs_in_only_the_server_and_user_of_its_flow() {
        let f = fixture();
        let alice = actor();
        let bob = actor();
        let hers = f.service.create(&alice, oauth_server()).await.unwrap();
        let his = f.service.create(&bob, oauth_server()).await.unwrap();
        let _alice_state = start(&f, &alice, hers.id()).await;
        let bob_state = start(&f, &bob, his.id()).await;
        f.auth.answer(Ok(tokens("bob-access", None)));
        let outcome = f.service.complete(&callback(&bob_state)).await;
        assert_eq!(outcome.server_id, Some(his.id()));
        assert_eq!(outcome.result, SignInResult::SignedIn);
        let hers = f.service.get(&alice, hers.id()).await.unwrap();
        assert_eq!(hers.sign_in, Some(SignInState::NeedsSignIn));
        let his = f.service.get(&bob, his.id()).await.unwrap().server;
        assert_eq!(session(&f, &his).access, Secret::new("bob-access"));
        assert!(
            f.service.sign_out(&alice, his.id()).await.is_err(),
            "not hers"
        );
    }

    #[tokio::test]
    async fn sends_the_access_token_and_refreshes_it_when_expired() {
        let f = fixture();
        let user = actor();
        let server = signed_in(&f, &user).await;
        let endpoint = f.service.endpoint(&user, &server).unwrap();
        assert_eq!(endpoint.headers, None);
        let tokens_source = endpoint.tokens.expect("tokens");
        assert_eq!(
            tokens_source.0.current().await.unwrap(),
            Secret::new("access-1")
        );

        f.clock.advance(Duration::hours(2));
        f.auth.answer(Ok(tokens("access-2", Some("refresh-2"))));
        assert_eq!(
            tokens_source.0.current().await.unwrap(),
            Secret::new("access-2")
        );
        let request = f.auth.requests().last().cloned().unwrap();
        assert_eq!(
            request.grant,
            Grant::Refresh {
                token: Secret::new("refresh-1")
            }
        );
        assert_eq!(request.resource, "https://mcp.example.com/mcp");
        let stored = f.service.get(&user, server.id()).await.unwrap().server;
        let session = session(&f, &stored);
        assert_eq!(session.access, Secret::new("access-2"));
        assert_eq!(session.refresh, Some(Secret::new("refresh-2")), "rotated");
    }

    #[tokio::test]
    async fn refreshes_once_on_a_rejected_token() {
        let f = fixture();
        let user = actor();
        let server = signed_in(&f, &user).await;
        let source = f.service.endpoint(&user, &server).unwrap().tokens.unwrap();
        f.auth.answer(Ok(tokens("access-2", None)));
        let fresh = source.0.rejected(&Secret::new("access-1")).await.unwrap();
        assert_eq!(fresh, Secret::new("access-2"));
        // A second request that saw the old token gets the new one without another refresh.
        let again = source.0.rejected(&Secret::new("access-1")).await.unwrap();
        assert_eq!(again, Secret::new("access-2"));
        assert_eq!(f.auth.requests().len(), 2, "one exchange, one refresh");
        let stored = f.service.get(&user, server.id()).await.unwrap().server;
        assert_eq!(
            session(&f, &stored).refresh,
            Some(Secret::new("refresh-1")),
            "kept without rotation"
        );
    }

    #[tokio::test]
    async fn a_rejected_refresh_needs_a_new_sign_in_and_a_failed_one_does_not() {
        let f = fixture();
        let user = actor();
        let server = signed_in(&f, &user).await;
        let source = f.service.endpoint(&user, &server).unwrap().tokens.unwrap();

        f.auth
            .answer(Err(DomainError::upstream("down", OAUTH_FAILED)));
        let error = source
            .0
            .rejected(&Secret::new("access-1"))
            .await
            .unwrap_err();
        assert_eq!(error.code(), OAUTH_FAILED);
        let details = f.service.get(&user, server.id()).await.unwrap();
        assert_eq!(details.sign_in, Some(SignInState::SignedIn), "kept");

        f.auth
            .answer(Err(DomainError::upstream("invalid_grant", OAUTH_REJECTED)));
        let error = source
            .0
            .rejected(&Secret::new("access-1"))
            .await
            .unwrap_err();
        assert_eq!(error.code(), SIGN_IN_REQUIRED);
        assert!(!error.to_string().contains("invalid_grant"));
        let details = f.service.get(&user, server.id()).await.unwrap();
        assert_eq!(details.sign_in, Some(SignInState::NeedsSignIn));
        let error = f.service.endpoint(&user, &details.server).unwrap_err();
        assert_eq!(error.code(), SIGN_IN_REQUIRED);
    }

    #[tokio::test]
    async fn without_a_refresh_token_an_expired_sign_in_shows_expired() {
        let f = fixture();
        let user = actor();
        let server = f.service.create(&user, oauth_server()).await.unwrap();
        let state = start(&f, &user, server.id()).await;
        f.auth.answer(Ok(tokens("access-1", None)));
        f.service.complete(&callback(&state)).await;
        let source = f
            .service
            .endpoint(
                &user,
                &f.service.get(&user, server.id()).await.unwrap().server,
            )
            .unwrap()
            .tokens
            .unwrap();
        f.clock.advance(Duration::hours(2));
        let details = f.service.get(&user, server.id()).await.unwrap();
        assert_eq!(details.sign_in, Some(SignInState::Expired));
        assert_eq!(
            source.0.current().await.unwrap_err().code(),
            SIGN_IN_REQUIRED
        );
        assert_eq!(
            f.service
                .endpoint(&user, &details.server)
                .unwrap_err()
                .code(),
            SIGN_IN_REQUIRED
        );
    }

    #[tokio::test]
    async fn signing_out_settings_and_url_changes_end_the_sign_in() {
        let f = fixture();
        let user = actor();
        let server = signed_in(&f, &user).await;
        // Other settings keep it.
        let renamed = f
            .service
            .update(
                &user,
                server.id(),
                ToolServerUpdate {
                    name: Some("Renamed".to_owned()),
                    ..ToolServerUpdate::default()
                },
            )
            .await
            .unwrap();
        let stored = f.service.get(&user, renamed.id()).await.unwrap();
        assert_eq!(stored.sign_in, Some(SignInState::SignedIn));

        let out = f.service.sign_out(&user, server.id()).await.unwrap();
        assert!(out.sign_in().is_none());
        assert_eq!(out.auth(), AuthMode::OAuth, "the mode stays");

        let server = signed_in(&f, &user).await;
        let moved = f
            .service
            .update(
                &user,
                server.id(),
                ToolServerUpdate {
                    url: Some("https://mcp.example.com/other".to_owned()),
                    ..ToolServerUpdate::default()
                },
            )
            .await
            .unwrap();
        assert!(moved.sign_in().is_none());
        let stored = f.service.get(&user, moved.id()).await.unwrap();
        assert_eq!(stored.sign_in, Some(SignInState::NeedsSignIn));
    }

    #[tokio::test]
    async fn switching_modes_drops_the_other_credentials() {
        let f = fixture();
        let user = actor();
        let server = signed_in(&f, &user).await;
        let headers = f
            .service
            .update(
                &user,
                server.id(),
                ToolServerUpdate {
                    credentials: Some(CredentialChange::Set {
                        bearer: Some("token".to_owned()),
                        headers: BTreeMap::new(),
                    }),
                    ..ToolServerUpdate::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(headers.auth(), AuthMode::Headers);
        assert!(headers.sign_in().is_none() && headers.client().is_none());
        let stored = f.service.get(&user, server.id()).await.unwrap();
        assert_eq!(stored.sign_in, None);
        assert!(stored.server.sign_in().is_none());

        let oauth = f
            .service
            .update(
                &user,
                server.id(),
                ToolServerUpdate {
                    auth: Some(AuthMode::OAuth),
                    ..ToolServerUpdate::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(oauth.auth(), AuthMode::OAuth);
        assert_eq!(oauth.headers_sealed(), None);

        let none = f
            .service
            .update(
                &user,
                server.id(),
                ToolServerUpdate {
                    auth: Some(AuthMode::None),
                    ..ToolServerUpdate::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(none.auth(), AuthMode::None);
    }

    #[tokio::test]
    async fn validates_modes_and_clients() {
        let f = fixture();
        let user = actor();
        let headers_without = f
            .service
            .create(
                &user,
                NewToolServer {
                    auth: Some(AuthMode::Headers),
                    ..oauth_server()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(field(&headers_without), Some("bearer"));
        let oauth_with_token = f
            .service
            .create(
                &user,
                NewToolServer {
                    bearer: Some("token".to_owned()),
                    ..oauth_server()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(field(&oauth_with_token), Some("authMode"));
        let client_without_oauth = f
            .service
            .create(
                &user,
                NewToolServer {
                    auth: None,
                    client: Some(ClientInput {
                        id: "x".to_owned(),
                        secret: None,
                    }),
                    ..oauth_server()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(field(&client_without_oauth), Some("oauthClient"));
        for id in ["", "  ", "two words", &"x".repeat(513)] {
            let error = f
                .service
                .create(
                    &user,
                    NewToolServer {
                        client: Some(ClientInput {
                            id: id.to_owned(),
                            secret: None,
                        }),
                        ..oauth_server()
                    },
                )
                .await
                .unwrap_err();
            assert_eq!(field(&error), Some("oauthClient"), "{id:?}");
        }
        let error = f
            .service
            .update(
                &user,
                f.service.create(&user, oauth_server()).await.unwrap().id(),
                ToolServerUpdate {
                    auth: Some(AuthMode::Headers),
                    ..ToolServerUpdate::default()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(field(&error), Some("bearer"));
    }

    #[tokio::test]
    async fn a_blank_client_secret_keeps_the_stored_one_for_the_same_id() {
        let f = fixture();
        let user = actor();
        let server = f
            .service
            .create(
                &user,
                NewToolServer {
                    client: Some(ClientInput {
                        id: " my-client ".to_owned(),
                        secret: Some("my-secret".to_owned()),
                    }),
                    ..oauth_server()
                },
            )
            .await
            .unwrap();
        let client = server.client().unwrap();
        assert_eq!(client.id, "my-client");
        let sealed = client.secret_sealed.clone().unwrap();
        let opened = f.cipher.open(&sealed, &server.client_context()).unwrap();
        assert_eq!(opened, Secret::new("my-secret"));

        let change = |id: &str| ToolServerUpdate {
            client: Some(ClientChange::Set(ClientInput {
                id: id.to_owned(),
                secret: Some(" ".to_owned()),
            })),
            ..ToolServerUpdate::default()
        };
        let kept = f
            .service
            .update(&user, server.id(), change("my-client"))
            .await
            .unwrap();
        assert_eq!(kept.client().unwrap().secret_sealed, Some(sealed));
        let other = f
            .service
            .update(&user, server.id(), change("other"))
            .await
            .unwrap();
        assert_eq!(other.client().unwrap().secret_sealed, None);
        let details = f.service.get(&user, server.id()).await.unwrap();
        assert_eq!(
            details.client_secret,
            crate::domains::provider::KeyState::Missing
        );
        let cleared = f
            .service
            .update(
                &user,
                server.id(),
                ToolServerUpdate {
                    client: Some(ClientChange::Clear),
                    ..ToolServerUpdate::default()
                },
            )
            .await
            .unwrap();
        assert!(cleared.client().is_none());
    }

    #[test]
    fn debug_hides_the_code_and_state() {
        let printed = format!("{:?}", callback("state-secret"));
        assert!(!printed.contains("state-secret"), "{printed}");
        assert!(!printed.contains("code-1"), "{printed}");
        let input = ClientInput {
            id: "client".to_owned(),
            secret: Some("client-secret".to_owned()),
        };
        assert!(!format!("{input:?}").contains("client-secret"));
    }
}
