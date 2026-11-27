//! A scripted `ToolAuth`: a fixed discovery, a fixed registration, and queued token answers.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use url::Url;

use crate::ports::tool_auth::{
    AuthServer, ClientAuth, Discovery, OAUTH_FAILED, OAuthClient, Registration, TokenRequest,
    TokenSet, ToolAuth,
};
use crate::shared::base_url::BaseUrl;
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::net::NetworkScope;
use crate::shared::secret::Secret;

/// Answers `discover` with [`ScriptedAuth::discovery`], `register` with client `registered-1`,
/// and `token` with the queued answers in order (an empty queue fails with `OAUTH_FAILED`).
/// Records every registration and token request.
pub struct ScriptedAuth {
    discovery: Mutex<Discovery>,
    tokens: Mutex<VecDeque<DomainResult<TokenSet>>>,
    registrations: Mutex<Vec<Registration>>,
    requests: Mutex<Vec<TokenRequest>>,
}

impl Default for ScriptedAuth {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptedAuth {
    pub fn new() -> Self {
        Self {
            discovery: Mutex::new(discovery()),
            tokens: Mutex::default(),
            registrations: Mutex::default(),
            requests: Mutex::default(),
        }
    }

    /// Changes what `discover` answers.
    pub fn edit(&self, change: impl FnOnce(&mut Discovery)) {
        change(&mut self.discovery.lock().unwrap());
    }

    /// Queues the answer of the next `token` call.
    pub fn answer(&self, result: DomainResult<TokenSet>) {
        self.tokens.lock().unwrap().push_back(result);
    }

    pub fn registrations(&self) -> Vec<Registration> {
        self.registrations.lock().unwrap().clone()
    }

    pub fn requests(&self) -> Vec<TokenRequest> {
        self.requests.lock().unwrap().clone()
    }
}

/// Tokens `access`, with `refresh` and a lifetime of one hour.
pub fn tokens(access: &str, refresh: Option<&str>) -> TokenSet {
    TokenSet {
        access: Secret::new(access),
        refresh: refresh.map(Secret::new),
        expires_in: Some(Duration::from_secs(3_600)),
        scope: None,
    }
}

/// An authorization server at `https://auth.example.com` with registration, PKCE, and `iss`.
pub fn discovery() -> Discovery {
    let url = |path: &str| Url::parse(&format!("https://auth.example.com{path}")).unwrap();
    Discovery {
        resource: "https://mcp.example.com/mcp".to_owned(),
        scopes: vec!["files:read".to_owned()],
        server: AuthServer {
            issuer: "https://auth.example.com".to_owned(),
            authorization_endpoint: url("/authorize"),
            token_endpoint: url("/token"),
            registration_endpoint: Some(url("/register")),
            client_metadata: false,
            iss_parameter: true,
            offline_access: true,
            auth_methods: Vec::new(),
        },
    }
}

#[async_trait]
impl ToolAuth for ScriptedAuth {
    async fn discover(&self, _url: &BaseUrl, _network: NetworkScope) -> DomainResult<Discovery> {
        Ok(self.discovery.lock().unwrap().clone())
    }

    async fn register(
        &self,
        _server: &AuthServer,
        registration: &Registration,
        _network: NetworkScope,
    ) -> DomainResult<OAuthClient> {
        self.registrations
            .lock()
            .unwrap()
            .push(registration.clone());
        Ok(OAuthClient {
            id: "registered-1".to_owned(),
            secret: None,
            auth: ClientAuth::None,
        })
    }

    async fn token(
        &self,
        request: &TokenRequest,
        _network: NetworkScope,
    ) -> DomainResult<TokenSet> {
        self.requests.lock().unwrap().push(request.clone());
        self.tokens.lock().unwrap().pop_front().unwrap_or_else(|| {
            Err(DomainError::upstream(
                "No scripted token answer.",
                OAUTH_FAILED,
            ))
        })
    }
}
