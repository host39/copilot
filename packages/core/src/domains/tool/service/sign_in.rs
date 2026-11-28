//! OAuth sign-in to tool servers (`docs/design/copilot.md`, Tool Server Sign-In): starting a
//! sign-in, its callback, the owner's confirmation, signing out, and the access tokens a run
//! sends, refreshed when they expire or the server answers `401`.
//!
//! Two attacks shape the flow. **Mix-up** (RFC 9700, 4.4): a malicious server's sign-in sends
//! the browser to an honest authorization server, whose code then comes back with the malicious
//! flow's `state`. Every sign-in with a pre-registered or registered client therefore comes back
//! to a redirect URI of its own authorization server (`<callback>/<issuer tag>`), and one with
//! this install's Client ID Metadata Document (a client id every authorization server accepts)
//! only where the authorization server sends `iss`, which must match. **Sign-in links between
//! users:** the callback is public, so the `state` alone does not say who came back. The callback
//! therefore exchanges the code but stores nothing on the server: it holds the tokens and shows
//! the returning browser a confirmation code, which only the flow's owner can send back
//! ([`ToolService::confirm`]). The web app does that on its own when the browser that comes back
//! is signed in to Copilot; the native apps ask for the code.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use futures_util::lock::Mutex as AsyncMutex;
use url::{Host, Url};
use uuid::Uuid;

use crate::domains::tool::auth::{
    AuthMode, Held, Pending, Secrets, Session, SignIn, SignInFlow, SignInState, challenge,
    confirmation, confirmation_hash, issuer_tag, state_hash,
};
use crate::domains::tool::entities::ToolServer;
use crate::domains::tool::ports::SignInRepository;
use crate::domains::tool::service::ToolService;
use crate::ports::tool_auth::{
    AccessTokens, ClientAuth, Discovery, Grant, OAUTH_REJECTED, OAuthClient, Registration,
    SIGN_IN_REQUIRED, TokenRequest, TokenSet, TokenSource, ToolAuth,
};
use crate::shared::actor::Actor;
use crate::shared::base_url::BaseUrl;
use crate::shared::constants::{SIGN_IN_CONFIRM_ATTEMPTS, SIGN_IN_FLOW_SECS};
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::ToolServerId;
use crate::shared::net::NetworkScope;
use crate::shared::secret::Secret;

/// The error code when this install cannot run sign-ins (`PUBLIC_URL` is unset).
pub const SIGN_IN_UNAVAILABLE: &str = "SIGN_IN_UNAVAILABLE";
/// The error code when the authorization server offers no way to get a client id: neither Client
/// ID Metadata Documents (with an `https` `PUBLIC_URL`, and `iss` in its answers) nor dynamic
/// registration.
pub const CLIENT_ID_REQUIRED: &str = "CLIENT_ID_REQUIRED";
/// The error code when the server's sign-in moved to another authorization server than the one
/// its pre-registered client was bound to: its secret is not sent there.
pub const CLIENT_ISSUER_CHANGED: &str = "CLIENT_ISSUER_CHANGED";

/// The client name sent with dynamic registration (and in the Client ID Metadata Document).
pub const CLIENT_NAME: &str = "Copilot";

/// Where sign-ins come back to, from `PUBLIC_URL`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignInConfig {
    /// `<PUBLIC_URL>/api/oauth/callback`: the redirect URI of sign-ins with the Client ID
    /// Metadata Document, and the base of the others' ([`SignInConfig::redirect_uri`]).
    pub callback: Url,
    /// `<PUBLIC_URL>/api/oauth/client`, the Client ID Metadata Document, when `PUBLIC_URL` is
    /// `https` and `OAUTH_CLIENT_METADATA` allows it; used as the client id by authorization
    /// servers that support such documents and send `iss`.
    pub client_metadata: Option<Url>,
}

impl SignInConfig {
    /// The redirect URI of sign-ins at `issuer` with a pre-registered or dynamically registered
    /// client: `<callback>/<issuer tag>`, so a code from another authorization server can never
    /// come back to it (a client registered there has other redirect URIs).
    pub fn redirect_uri(&self, issuer: &str) -> Url {
        self.arrival(Some(&issuer_tag(issuer)))
    }

    /// The redirect URI a callback came back to: the callback itself, or `<callback>/<tag>`.
    fn arrival(&self, tag: Option<&str>) -> Url {
        let mut url = self.callback.clone();
        if let Some(tag) = tag
            && let Ok(mut segments) = url.path_segments_mut()
        {
            segments.push(tag);
        }
        url
    }
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
    /// The last segment of the path it came back to (`/api/oauth/callback/<tag>`); `None` for
    /// `/api/oauth/callback`.
    pub tag: Option<String>,
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
            .field("tag", &self.tag)
            .field("iss", &self.iss)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

/// How a callback ended, for the page the browser is sent to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignInOutcome {
    /// The server the sign-in was for, when the `state` named a live flow.
    pub server_id: Option<ToolServerId>,
    pub result: SignInResult,
    /// The code the browser shows for [`SignInResult::Confirm`].
    pub confirmation: Option<Secret>,
}

impl SignInOutcome {
    fn ended(server_id: Option<ToolServerId>, result: SignInResult) -> Self {
        Self {
            server_id,
            result,
            confirmation: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignInResult {
    /// The code was exchanged and the tokens are held: the flow's owner confirms them with the
    /// code shown to the browser ([`ToolService::confirm`]).
    Confirm,
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
            Self::Confirm => "confirm",
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

    /// The redirect URI to register for the server's pre-registered client, once its first
    /// sign-in bound it to an authorization server.
    pub fn client_redirect(&self, server: &ToolServer) -> Option<Url> {
        let issuer = server.client()?.issuer.as_deref()?;
        Some(self.sign_in_config()?.redirect_uri(issuer))
    }

    /// Starts signing in to an `OAuth` server: discovers its authorization server, gets a client
    /// id (the pre-registered one, bound to that authorization server on its first sign-in; else
    /// this install's Client ID Metadata Document when the authorization server supports those
    /// and sends `iss`; else dynamic registration) with its redirect URI, and stores the PKCE
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
        let discovery = oauth.auth.discover(http_url(&server)?, network).await?;
        let scope = scope(&discovery);
        let (client, redirect_uri) = self
            .oauth_client(oauth, &server, &discovery, scope.clone(), network)
            .await?;
        let secrets = Secrets::generate()?;
        let state = secrets.state.text().unwrap_or_default().to_owned();
        let now = self.clock.now();
        let pending = Pending {
            verifier: secrets.verifier,
            redirect_uri,
            issuer: discovery.server.issuer.clone(),
            iss_parameter: discovery.server.iss_parameter,
            token_endpoint: discovery.server.token_endpoint.clone(),
            client,
            resource: discovery.resource.clone(),
            scope: scope.clone(),
            url: http_url(&server)?.as_str().to_owned(),
            private: network == NetworkScope::Private,
        };
        let mut flow = SignInFlow {
            id: Uuid::now_v7(),
            user_id: actor.user_id,
            server_id: id,
            state_hash: state_hash(&state),
            sealed: Vec::new(),
            confirm_hash: None,
            attempts: 0,
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

    /// Takes the authorization server's redirect. The `state` must name a stored flow, which
    /// this uses up whatever happens next; the flow must still be live; the redirect must have
    /// come back to the flow's own redirect URI (its authorization server's); a present `iss`
    /// must equal the recorded issuer, and an absent one is refused when the issuer advertises
    /// it (RFC 9207); the server must still be the user's, in `OAuth` mode, at the same URL.
    /// Then the code is exchanged and the tokens are held for `SIGN_IN_FLOW_SECS`, not stored:
    /// the outcome is [`SignInResult::Confirm`] with the code that the flow's owner confirms
    /// them with ([`ToolService::confirm`]). Never fails: the outcome says what happened, and
    /// the authorization server's error text is never passed on.
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
        if pending.redirect_uri != oauth.config.arrival(callback.tag.as_deref()) {
            return Err(failed(server_id, "redirect-mismatch"));
        }
        let code = accepted(callback, &pending, flow.server_id)?;
        let server = match self.servers.get(flow.user_id, flow.server_id).await {
            Ok(Some(server))
                if server.auth() == AuthMode::OAuth
                    && server.url().is_some_and(|url| url.as_str() == pending.url) =>
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
        let code = self.hold(oauth, &flow, pending, &tokens).await?;
        tracing::info!(server_id = %server.id(), user_id = %flow.user_id, "core.tools.sign-in.returned");
        Ok(SignInOutcome {
            server_id,
            result: SignInResult::Confirm,
            confirmation: Some(code),
        })
    }

    /// Replaces the used-up flow with a held one: the session the exchange gave, sealed, under a
    /// new confirmation code (stored as its hash) and a `state` nobody knows. Returns the code.
    async fn hold(
        &self,
        oauth: &OAuth,
        flow: &SignInFlow,
        pending: Pending,
        tokens: &TokenSet,
    ) -> Result<Secret, SignInOutcome> {
        let server_id = Some(flow.server_id);
        let held = Held {
            session: Session {
                issuer: pending.issuer,
                token_endpoint: pending.token_endpoint,
                client: pending.client,
                resource: pending.resource,
                scope: pending.scope,
                access: Secret::new(""),
                refresh: None,
            }
            .renewed(tokens.clone()),
            expires_at: self.expiry(tokens),
            url: pending.url,
        };
        let random = || failed(server_id, "random");
        let code = confirmation().map_err(|_| random())?;
        let unknown = Secrets::generate().map_err(|_| random())?;
        let now = self.clock.now();
        let mut kept = SignInFlow {
            id: Uuid::now_v7(),
            user_id: flow.user_id,
            server_id: flow.server_id,
            state_hash: state_hash(unknown.state.text().unwrap_or_default()),
            sealed: Vec::new(),
            confirm_hash: Some(confirmation_hash(code.text().unwrap_or_default())),
            attempts: 0,
            created_at: now,
            expires_at: now + Duration::seconds(SIGN_IN_FLOW_SECS),
        };
        kept.sealed = self
            .cipher
            .seal(&held.to_json(), &kept.context())
            .map_err(|_| failed(server_id, "seal"))?;
        oauth
            .flows
            .insert(&kept)
            .await
            .map_err(|_| failed(server_id, "storage"))?;
        Ok(code)
    }

    /// Finishes a sign-in that came back ([`ToolService::complete`]): `code` must be the one the
    /// returning browser was shown, sent by the server's owner (anyone else gets `NOT_FOUND`),
    /// within `SIGN_IN_FLOW_SECS` of the callback and within `SIGN_IN_CONFIRM_ATTEMPTS` tries.
    /// Then the held tokens are stored sealed for the server, if it still has the URL the
    /// sign-in started with and `OAuth` mode.
    pub async fn confirm(
        &self,
        actor: &Actor,
        id: ToolServerId,
        code: &str,
    ) -> DomainResult<ToolServer> {
        let oauth = self.oauth()?;
        let server = self.load(actor, id).await?;
        let waiting = || {
            DomainError::validation(
                "No sign-in of this server is waiting for a code. Start it again with Sign in.",
            )
            .with_field("code")
        };
        let flow = oauth
            .flows
            .held(actor.user_id, id)
            .await?
            .ok_or_else(waiting)?;
        if flow.expires_at <= self.clock.now() {
            oauth.flows.remove(flow.id).await?;
            return Err(waiting());
        }
        // Every try counts before it is compared, so parallel guesses share the limit.
        let attempt = oauth.flows.attempt(flow.id).await?;
        if attempt == 0 {
            return Err(waiting());
        }
        let matches = flow.confirm_hash.as_deref() == Some(confirmation_hash(code).as_slice());
        if !matches || attempt > SIGN_IN_CONFIRM_ATTEMPTS {
            tracing::warn!(server_id = %id, attempt, "core.tools.sign-in.confirm-refused");
            if attempt >= SIGN_IN_CONFIRM_ATTEMPTS {
                oauth.flows.remove(flow.id).await?;
                return Err(DomainError::validation(
                    "Too many wrong codes. Start the sign-in again with Sign in.",
                )
                .with_field("code"));
            }
            return Err(DomainError::validation(
                "That code does not match the one the browser showed.",
            )
            .with_field("code"));
        }
        if !oauth.flows.remove(flow.id).await? {
            return Err(waiting());
        }
        let held = self
            .cipher
            .open(&flow.sealed, &flow.context())
            .and_then(|secret| Held::parse(&secret))?;
        let changed = || {
            DomainError::validation(
                "The server's address or authentication changed since the sign-in started. Sign in again.",
            )
        };
        let url = http_url(&server)?;
        if server.auth() != AuthMode::OAuth || url.as_str() != held.url {
            return Err(changed());
        }
        let sign_in = self.seal_sign_in(&server, &held.session, held.expires_at)?;
        if !self
            .servers
            .save_sign_in(actor.user_id, id, url, Some(&sign_in), None)
            .await?
        {
            return Err(changed());
        }
        // A new sign-in may fix what failed: the next turn lists the server again.
        self.failures().remove(&id);
        tracing::info!(server_id = %id, user_id = %actor.user_id, "core.tools.sign-in.completed");
        self.load(actor, id).await
    }

    /// Takes the flow the callback's `state` names, which uses it up; it must still be live.
    async fn live(&self, oauth: &OAuth, callback: &Callback) -> Result<SignInFlow, SignInOutcome> {
        let flow = match oauth.flows.take(&state_hash(&callback.state)).await {
            Ok(Some(flow)) => flow,
            Ok(None) => {
                tracing::warn!("core.tools.sign-in.unknown-state");
                return Err(SignInOutcome::ended(None, SignInResult::Expired));
            }
            Err(error) => {
                tracing::warn!(error = %error, "core.tools.sign-in.flow-failed");
                return Err(failed(None, "storage"));
            }
        };
        if flow.expires_at <= self.clock.now() {
            tracing::info!(server_id = %flow.server_id, "core.tools.sign-in.expired");
            return Err(SignInOutcome::ended(
                Some(flow.server_id),
                SignInResult::Expired,
            ));
        }
        Ok(flow)
    }

    /// Forgets the server's sign-in (the authorization server keeps its grant until it
    /// expires; revoke it there to end it everywhere).
    pub async fn sign_out(&self, actor: &Actor, id: ToolServerId) -> DomainResult<ToolServer> {
        let server = self.load(actor, id).await?;
        if let Some(url) = server.url() {
            self.servers
                .save_sign_in(actor.user_id, id, url, None, None)
                .await?;
        }
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

    /// The client id of a sign-in, in the order the MCP specification gives, with the redirect
    /// URI the sign-in comes back to. A pre-registered client is bound to the authorization
    /// server its first sign-in finds and refused at any other, so its secret only ever goes to
    /// the token endpoint of the server it was entered for. This install's Client ID Metadata
    /// Document is a client id any authorization server accepts, so it is used only where the
    /// answers carry `iss` (checked at the callback); elsewhere a code that another
    /// authorization server sent to the shared redirect URI could not be told apart.
    async fn oauth_client(
        &self,
        oauth: &OAuth,
        server: &ToolServer,
        discovery: &Discovery,
        scope: Option<String>,
        network: NetworkScope,
    ) -> DomainResult<(OAuthClient, Url)> {
        let issuer = &discovery.server.issuer;
        let redirect_uri = oauth.config.redirect_uri(issuer);
        let methods = &discovery.server.auth_methods;
        if let Some(config) = server.client() {
            match &config.issuer {
                Some(bound) if bound != issuer => {
                    tracing::warn!(server_id = %server.id(), "core.tools.sign-in.client-issuer-changed");
                    return Err(DomainError::business_rule(
                        format!(
                            "This server's sign-in moved to another authorization server ({issuer}) than the one its client ID belongs to. If that is right, save the client again in the server's settings."
                        ),
                        CLIENT_ISSUER_CHANGED,
                    ));
                }
                Some(_) => {}
                None => {
                    let bound = self
                        .servers
                        .bind_client(server.user_id(), server.id(), &config.id, issuer)
                        .await?;
                    if !bound {
                        return Err(DomainError::validation(
                            "The server's client changed while the sign-in started. Try again.",
                        )
                        .with_field("oauthClient"));
                    }
                }
            }
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
            let client = OAuthClient {
                id: config.id.clone(),
                secret,
                auth,
            };
            return Ok((client, redirect_uri));
        }
        if discovery.server.client_metadata
            && discovery.server.iss_parameter
            && let Some(document) = &oauth.config.client_metadata
        {
            let client = OAuthClient {
                id: document.as_str().to_owned(),
                secret: None,
                auth: ClientAuth::None,
            };
            return Ok((client, oauth.config.callback.clone()));
        }
        if discovery.server.registration_endpoint.is_some() {
            let registration = Registration {
                client_name: CLIENT_NAME.to_owned(),
                redirect_uri: redirect_uri.clone(),
                application_type: if loopback(&redirect_uri) {
                    "native"
                } else {
                    "web"
                },
                scope,
            };
            let client = oauth
                .auth
                .register(&discovery.server, &registration, network)
                .await?;
            return Ok((client, redirect_uri));
        }
        Err(DomainError::business_rule(
            format!(
                "This tool server's sign-in needs a client ID: register Copilot with its authorization server ({issuer}) with the redirect URI {redirect_uri}, then enter the client ID in the server's settings."
            ),
            CLIENT_ID_REQUIRED,
        ))
    }

    /// When the access token of `tokens`, answered now, expires (`None` when the server did not
    /// say).
    fn expiry(&self, tokens: &TokenSet) -> Option<DateTime<Utc>> {
        tokens
            .expires_in
            .and_then(|lifetime| Duration::from_std(lifetime).ok())
            .and_then(|lifetime| self.clock.now().checked_add_signed(lifetime))
    }

    /// Seals `session` for `server`'s row, with when its access token expires.
    fn seal_sign_in(
        &self,
        server: &ToolServer,
        session: &Session,
        expires_at: Option<DateTime<Utc>>,
    ) -> DomainResult<SignIn> {
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
            Err(SignInOutcome::ended(Some(server), SignInResult::Denied))
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
    SignInOutcome::ended(server_id, SignInResult::Failed)
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
        let next = service.seal_sign_in(server, &renewed, service.expiry(&tokens))?;
        let saved = service
            .servers
            .save_sign_in(
                self.actor.user_id,
                id,
                http_url(server)?,
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
                http_url(server)?,
                None,
                Some(&sign_in.sealed),
            )
            .await?;
        Ok(())
    }
}

/// The URL of an `OAuth` server. Only Streamable HTTP servers sign in (a stdio server's program
/// gets its credentials from its environment), so a stdio server fails as not signed in.
fn http_url(server: &ToolServer) -> DomainResult<&BaseUrl> {
    server.url().ok_or_else(|| sign_in_required(server))
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
    const ISSUER: &str = "https://auth.example.com";

    /// The redirect URI of sign-ins at the scripted authorization server.
    fn redirect() -> String {
        format!("{REDIRECT}/{}", issuer_tag(ISSUER))
    }

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
                callback: Url::parse(REDIRECT).unwrap(),
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

    /// A callback to the scripted authorization server's redirect URI.
    fn callback(state: &str) -> Callback {
        Callback {
            tag: Some(issuer_tag(ISSUER)),
            state: state.to_owned(),
            code: Some("code-1".to_owned()),
            iss: Some(ISSUER.to_owned()),
            error: None,
        }
    }

    /// The code the browser is shown after a callback that came back.
    fn shown(outcome: &SignInOutcome) -> String {
        assert_eq!(outcome.result, SignInResult::Confirm, "{outcome:?}");
        outcome
            .confirmation
            .as_ref()
            .and_then(Secret::text)
            .expect("a confirmation code")
            .to_owned()
    }

    async fn signed_in(f: &Fixture, actor: &Actor) -> ToolServer {
        let server = f.service.create(actor, oauth_server()).await.unwrap();
        let state = start(f, actor, server.id()).await;
        f.auth.answer(Ok(tokens("access-1", Some("refresh-1"))));
        let code = shown(&f.service.complete(&callback(&state)).await);
        f.service
            .confirm(actor, server.id(), &code)
            .await
            .expect("confirmed")
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
        assert_eq!(
            query["redirect_uri"],
            redirect(),
            "its authorization server's own"
        );
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
        assert_eq!(registration.redirect_uri.as_str(), redirect());

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
        assert_eq!(query(&started.url)["redirect_uri"], redirect());

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
        assert_eq!(
            query(&started.url)["redirect_uri"],
            REDIRECT,
            "the document's redirect URI"
        );
        assert!(f.auth.registrations().is_empty());

        // The document is a client id every authorization server takes: only where the answer
        // carries `iss`, so a code from another one cannot pass for this one's.
        f.auth
            .edit(|discovery| discovery.server.iss_parameter = false);
        let started = f.service.sign_in(&user, other.id()).await.unwrap();
        assert_eq!(query(&started.url)["client_id"], "registered-1");
        assert_eq!(query(&started.url)["redirect_uri"], redirect());
        f.auth
            .edit(|discovery| discovery.server.iss_parameter = true);

        f.auth.edit(|discovery| {
            discovery.server.client_metadata = false;
            discovery.server.registration_endpoint = None;
        });
        let error = f.service.sign_in(&user, other.id()).await.unwrap_err();
        assert_eq!(error.code(), CLIENT_ID_REQUIRED);
        assert!(
            error.to_string().contains(&redirect()) && error.to_string().contains(ISSUER),
            "names the redirect URI to register: {error}"
        );
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
    async fn a_callback_holds_tokens_until_the_owner_confirms_them() {
        let f = fixture();
        let user = actor();
        let server = f.service.create(&user, oauth_server()).await.unwrap();
        let state = start(&f, &user, server.id()).await;
        f.auth.answer(Ok(tokens("access-1", Some("refresh-1"))));
        let outcome = f.service.complete(&callback(&state)).await;
        assert_eq!(outcome.server_id, Some(server.id()));
        let code = shown(&outcome);
        let request = &f.auth.requests()[0];
        let Grant::Code {
            code: exchanged,
            verifier,
            redirect_uri,
        } = &request.grant
        else {
            panic!("a code grant");
        };
        assert_eq!(exchanged, &Secret::new("code-1"));
        assert_eq!(redirect_uri.as_str(), redirect());
        assert_eq!(request.resource, "https://mcp.example.com/mcp");
        assert_eq!(request.client.id, "registered-1");
        assert_eq!(verifier.text().unwrap().len(), 43);

        // Nothing is stored on the server until its owner sends the code back.
        let details = f.service.get(&user, server.id()).await.unwrap();
        assert_eq!(details.sign_in, Some(SignInState::NeedsSignIn));
        let held = f.store.tables().sign_in_flows.clone();
        let flow = held.values().next().unwrap();
        assert_eq!(
            flow.confirm_hash.as_deref(),
            Some(confirmation_hash(&code).as_slice())
        );
        assert!(!String::from_utf8_lossy(&flow.sealed).contains(&code));

        let stored = f.service.confirm(&user, server.id(), &code).await.unwrap();
        let details = f.service.get(&user, server.id()).await.unwrap();
        assert_eq!(details.sign_in, Some(SignInState::SignedIn));
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
        assert!(f.store.tables().sign_in_flows.is_empty(), "used up");
        let again = f
            .service
            .confirm(&user, server.id(), &code)
            .await
            .unwrap_err();
        assert_eq!(field(&again), Some("code"));

        // The same state again: used up.
        let replay = f.service.complete(&callback(&state)).await;
        assert_eq!(replay.result, SignInResult::Expired);
        assert_eq!(replay.server_id, None);
        assert_eq!(f.auth.requests().len(), 1, "no second exchange");
    }

    /// A sign-in link one user sends another (RFC 9700, 4.7): the receiver's browser comes back
    /// with the receiver's code, but only the sender's server is waiting, and only the code the
    /// receiver's browser shows confirms it. The sender never sees it; the receiver cannot use it.
    #[tokio::test]
    async fn a_sign_in_link_sent_to_another_user_signs_in_nobody() {
        let f = fixture();
        let sender = actor();
        let receiver = actor();
        let server = f.service.create(&sender, oauth_server()).await.unwrap();
        let state = start(&f, &sender, server.id()).await;
        f.auth.answer(Ok(tokens("receivers-access", None)));
        let code = shown(&f.service.complete(&callback(&state)).await);

        let error = f
            .service
            .confirm(&receiver, server.id(), &code)
            .await
            .unwrap_err();
        assert_eq!(
            error.code(),
            "ENTITY_NOT_FOUND",
            "not the receiver's server"
        );
        for attempt in 1..SIGN_IN_CONFIRM_ATTEMPTS {
            let error = f
                .service
                .confirm(&sender, server.id(), "BCDF-GHJK")
                .await
                .unwrap_err();
            assert_eq!(field(&error), Some("code"));
            assert!(error.to_string().contains("does not match"), "{attempt}");
        }
        let error = f
            .service
            .confirm(&sender, server.id(), "BCDF-GHJK")
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("Too many wrong codes"),
            "{error}"
        );
        let error = f
            .service
            .confirm(&sender, server.id(), &code)
            .await
            .unwrap_err();
        assert_eq!(field(&error), Some("code"), "the flow is gone");
        let details = f.service.get(&sender, server.id()).await.unwrap();
        assert_eq!(details.sign_in, Some(SignInState::NeedsSignIn));
    }

    #[tokio::test]
    async fn held_tokens_expire_and_follow_the_server() {
        let f = fixture();
        let user = actor();
        let server = f.service.create(&user, oauth_server()).await.unwrap();
        let state = start(&f, &user, server.id()).await;
        f.auth.answer(Ok(tokens("access-1", None)));
        let code = shown(&f.service.complete(&callback(&state)).await);
        f.clock.advance(Duration::seconds(SIGN_IN_FLOW_SECS));
        let error = f
            .service
            .confirm(&user, server.id(), &code)
            .await
            .unwrap_err();
        assert_eq!(field(&error), Some("code"));

        // Lower case and spaces count as the same code; a URL change since the start does not.
        let state = start(&f, &user, server.id()).await;
        f.auth.answer(Ok(tokens("access-1", None)));
        let code = shown(&f.service.complete(&callback(&state)).await);
        f.service
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
        let typed = code.to_lowercase().replace('-', " ");
        let error = f
            .service
            .confirm(&user, server.id(), &typed)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("changed since"), "{error}");
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

        // Back to another redirect URI than its authorization server's: the shared callback (an
        // authorization server that took this install's client document), or another one's.
        for tag in [None, Some(issuer_tag("https://evil.example.com"))] {
            let state = start(&f, &user, server.id()).await;
            let mut elsewhere = callback(&state);
            elsewhere.tag = tag;
            assert_eq!(
                f.service.complete(&elsewhere).await.result,
                SignInResult::Failed
            );
        }

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
            SignInResult::Confirm
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
        let code = shown(&outcome);
        assert!(f.service.confirm(&alice, hers.id(), &code).await.is_err());
        f.service.confirm(&bob, his.id(), &code).await.unwrap();
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
        let code = shown(&f.service.complete(&callback(&state)).await);
        f.service.confirm(&user, server.id(), &code).await.unwrap();
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

    /// The reviewer's mix-up: a malicious server's discovery names its own issuer and token
    /// endpoint, says it takes client documents, and leaves `iss` out. Its sign-in gets a
    /// registered client and its own redirect URI; a code an honest authorization server sent to
    /// the shared callback (this install's client document) with the malicious `state` and no
    /// `iss` is refused, and nothing is sent to the malicious token endpoint.
    #[tokio::test]
    async fn a_code_from_another_authorization_server_is_refused() {
        let f = fixture();
        let user = actor();
        let server = f.service.create(&user, oauth_server()).await.unwrap();
        let evil = "https://attacker.example.org";
        f.auth.edit(|discovery| {
            discovery.server.issuer = evil.to_owned();
            discovery.server.token_endpoint = Url::parse(&format!("{evil}/token")).unwrap();
            discovery.server.client_metadata = true;
            discovery.server.iss_parameter = false;
        });
        let started = f.service.sign_in(&user, server.id()).await.unwrap();
        let query = query(&started.url);
        assert_eq!(
            query["client_id"], "registered-1",
            "not the shared document"
        );
        assert_eq!(
            query["redirect_uri"],
            format!("{REDIRECT}/{}", issuer_tag(evil))
        );
        let honest = Callback {
            tag: None,
            state: query["state"].clone(),
            code: Some("code-from-honest-as".to_owned()),
            iss: None,
            error: None,
        };
        assert_eq!(
            f.service.complete(&honest).await.result,
            SignInResult::Failed
        );
        assert!(f.auth.requests().is_empty(), "no code went to the attacker");

        // With `iss` advertised the document is used, and a code without `iss`, or with the
        // honest one's, is refused at the shared callback too.
        f.auth
            .edit(|discovery| discovery.server.iss_parameter = true);
        for iss in [None, Some(ISSUER.to_owned())] {
            let started = f.service.sign_in(&user, server.id()).await.unwrap();
            let query = super::tests::query(&started.url);
            assert_eq!(
                query["client_id"],
                "https://copilot.example.com/api/oauth/client"
            );
            let honest = Callback {
                tag: None,
                state: query["state"].clone(),
                code: Some("code-from-honest-as".to_owned()),
                iss,
                error: None,
            };
            assert_eq!(
                f.service.complete(&honest).await.result,
                SignInResult::Failed
            );
        }
        assert!(f.auth.requests().is_empty());
    }

    /// A pre-registered client's secret goes only to the authorization server of its first
    /// sign-in, and an origin change forgets the client.
    #[tokio::test]
    async fn a_client_secret_stays_with_its_authorization_server() {
        let f = fixture();
        let user = actor();
        let with_client = |url: &str| NewToolServer {
            url: url.to_owned(),
            client: Some(ClientInput {
                id: "org-app".to_owned(),
                secret: Some("org-app-secret".to_owned()),
            }),
            ..oauth_server()
        };
        let server = f
            .service
            .create(&user, with_client("https://mcp.example.com/mcp"))
            .await
            .unwrap();
        let bound = |f: &Fixture, id| {
            f.store.tables().tool_servers[&id]
                .client()
                .and_then(|client| client.issuer.clone())
        };
        assert_eq!(bound(&f, server.id()), None);
        let state = start(&f, &user, server.id()).await;
        assert_eq!(
            bound(&f, server.id()).as_deref(),
            Some(ISSUER),
            "bound at its first start"
        );
        f.auth.answer(Ok(tokens("access-1", None)));
        f.service.complete(&callback(&state)).await;
        assert_eq!(f.auth.requests().len(), 1);

        // The server now names another authorization server: the secret is not sent there.
        let evil = "https://attacker.example.org";
        f.auth.edit(|discovery| {
            discovery.server.issuer = evil.to_owned();
            discovery.server.token_endpoint = Url::parse(&format!("{evil}/token")).unwrap();
        });
        let error = f.service.sign_in(&user, server.id()).await.unwrap_err();
        assert_eq!(error.code(), CLIENT_ISSUER_CHANGED);
        assert!(error.to_string().contains(evil), "{error}");
        assert_eq!(f.auth.requests().len(), 1, "no token request");

        // Saving the client again is the user's word that it belongs there.
        f.service
            .update(
                &user,
                server.id(),
                ToolServerUpdate {
                    client: Some(ClientChange::Set(ClientInput {
                        id: "org-app".to_owned(),
                        secret: None,
                    })),
                    ..ToolServerUpdate::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(bound(&f, server.id()), None);
        f.service.sign_in(&user, server.id()).await.unwrap();
        assert_eq!(bound(&f, server.id()).as_deref(), Some(evil));

        // Another origin forgets the client with its secret, as it forgets headers; the same
        // origin keeps it.
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
        assert!(moved.client().is_some(), "same origin");
        let moved = f
            .service
            .update(
                &user,
                server.id(),
                ToolServerUpdate {
                    url: Some("https://other-origin.example.net/mcp".to_owned()),
                    ..ToolServerUpdate::default()
                },
            )
            .await
            .unwrap();
        assert!(moved.client().is_none(), "another origin");
        let kept = f
            .service
            .update(
                &user,
                server.id(),
                ToolServerUpdate {
                    url: Some("https://mcp.example.com/mcp".to_owned()),
                    client: Some(ClientChange::Set(ClientInput {
                        id: "new-app".to_owned(),
                        secret: None,
                    })),
                    ..ToolServerUpdate::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            kept.client().unwrap().id,
            "new-app",
            "set in the same update"
        );
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
