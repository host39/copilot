//! MCP authorization port: the OAuth 2.1 requests a tool server sign-in makes
//! (`docs/design/copilot.md`, Tool Server Sign-In).
//!
//! `copilot-mcp` implements [`ToolAuth`] over the hardened outbound clients: discovery (RFC 9728
//! protected resource metadata, then RFC 8414 or OpenID Connect authorization server metadata),
//! dynamic client registration (RFC 7591), and the token endpoint (authorization code with PKCE,
//! and refresh). Core decides everything else: which client id to use, the PKCE verifier and the
//! `state`, the authorization URL, what is stored (sealed), and when to refresh.
//!
//! [`AccessTokens`] goes the other way: core hands it to the MCP adapter inside a
//! [`crate::ToolEndpoint`], and the adapter asks it for the access token when it connects and for
//! a new one when the server answers `401`.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use url::Url;

use crate::shared::base_url::BaseUrl;
use crate::shared::errors::DomainResult;
use crate::shared::net::NetworkScope;
use crate::shared::secret::Secret;

/// Discovery found no usable authorization server, or its metadata breaks a rule the client must
/// enforce (issuer mismatch, no PKCE `S256`, an endpoint that is not `https`).
pub const OAUTH_DISCOVERY: &str = "OAUTH_DISCOVERY";
/// The authorization server refused dynamic client registration.
pub const OAUTH_REGISTRATION: &str = "OAUTH_REGISTRATION";
/// The token endpoint answered with an OAuth error (`invalid_grant`, `invalid_client`, and the
/// like): the code or the refresh token is no good, and only a new sign-in helps.
pub const OAUTH_REJECTED: &str = "OAUTH_REJECTED";
/// Any other failure of an OAuth request: no connection, a timeout, a server error, or a reply
/// that is not OAuth. A refresh that fails this way keeps the sign-in.
pub const OAUTH_FAILED: &str = "OAUTH_FAILED";
/// The outbound policy refused an OAuth URL (a private address for a public-only account, plain
/// `http`, credentials in the URL).
pub const OAUTH_BLOCKED: &str = "OAUTH_BLOCKED";
/// The server has no usable sign-in: never signed in, signed out, or its refresh was rejected.
pub const SIGN_IN_REQUIRED: &str = "SIGN_IN_REQUIRED";

#[async_trait]
pub trait ToolAuth: Send + Sync {
    /// Finds the authorization server of the MCP server at `url`: an unauthenticated request
    /// for its `401` challenge (`resource_metadata`, `scope`), its protected resource metadata
    /// (the challenge's URL, else the well-known URIs), and the first listed authorization
    /// server's metadata (RFC 8414 and OpenID Connect discovery URLs in the order the MCP
    /// specification gives), with the issuer and PKCE checks.
    async fn discover(&self, url: &BaseUrl, network: NetworkScope) -> DomainResult<Discovery>;

    /// Registers a client with `server`'s registration endpoint (RFC 7591).
    async fn register(
        &self,
        server: &AuthServer,
        registration: &Registration,
        network: NetworkScope,
    ) -> DomainResult<OAuthClient>;

    /// Calls `request.endpoint` (the token endpoint) for an access token. An OAuth error answer
    /// fails with [`OAUTH_REJECTED`], every other failure with [`OAUTH_FAILED`] or
    /// [`OAUTH_BLOCKED`].
    async fn token(&self, request: &TokenRequest, network: NetworkScope) -> DomainResult<TokenSet>;
}

/// What discovery found for one MCP server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Discovery {
    /// The `resource` parameter (RFC 8707) of every authorization and token request: the
    /// protected resource metadata's `resource` when it covers the server's URL, else the URL.
    pub resource: String,
    /// The scopes to request: the challenge's `scope`, else the metadata's `scopes_supported`;
    /// empty means no `scope` parameter.
    pub scopes: Vec<String>,
    pub server: AuthServer,
}

/// The validated metadata of an authorization server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthServer {
    /// Identical to the issuer the metadata URL was built from.
    pub issuer: String,
    /// Opened in the user's browser: `https` (plain `http` only for a private-network account).
    pub authorization_endpoint: Url,
    pub token_endpoint: Url,
    pub registration_endpoint: Option<Url>,
    /// `client_id_metadata_document_supported`.
    pub client_metadata: bool,
    /// `authorization_response_iss_parameter_supported` (RFC 9207).
    pub iss_parameter: bool,
    /// `scopes_supported` lists `offline_access`, which asks for a refresh token.
    pub offline_access: bool,
    /// `token_endpoint_auth_methods_supported`; empty means the default
    /// (`client_secret_basic`).
    pub auth_methods: Vec<String>,
}

/// A dynamic client registration request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Registration {
    pub client_name: String,
    pub redirect_uri: Url,
    /// `native` for a loopback redirect URI, else `web` (the MCP specification requires one).
    pub application_type: &'static str,
    pub scope: Option<String>,
}

/// How the client authenticates at the token endpoint.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ClientAuth {
    /// A public client: `client_id` in the form body.
    #[default]
    None,
    /// `client_secret_basic`: HTTP Basic with the id and secret.
    Basic,
    /// `client_secret_post`: id and secret in the form body.
    Post,
}

impl ClientAuth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Basic => "client_secret_basic",
            Self::Post => "client_secret_post",
        }
    }

    /// The method a registration answer or a metadata entry names; unknown ones are `None`.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "client_secret_basic" => Some(Self::Basic),
            "client_secret_post" => Some(Self::Post),
            _ => None,
        }
    }
}

/// An OAuth client: its id, and the secret of a confidential one. `Debug` hides the secret.
#[derive(Clone, Eq, PartialEq)]
pub struct OAuthClient {
    pub id: String,
    pub secret: Option<Secret>,
    pub auth: ClientAuth,
}

impl fmt::Debug for OAuthClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthClient")
            .field("id", &self.id)
            .field("secret", &self.secret)
            .field("auth", &self.auth)
            .finish()
    }
}

/// A token endpoint request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenRequest {
    pub endpoint: Url,
    pub client: OAuthClient,
    /// RFC 8707 `resource`.
    pub resource: String,
    pub grant: Grant,
}

/// The grant a token request presents. `Debug` hides the code, verifier, and token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Grant {
    /// `authorization_code` with the PKCE verifier and the redirect URI of the authorization
    /// request.
    Code {
        code: Secret,
        verifier: Secret,
        redirect_uri: Url,
    },
    /// `refresh_token`.
    Refresh { token: Secret },
}

/// A token endpoint answer. `Debug` hides the tokens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenSet {
    pub access: Secret,
    /// `None` keeps the refresh token the request used (no rotation).
    pub refresh: Option<Secret>,
    pub expires_in: Option<Duration>,
    /// The granted scope, when the server says.
    pub scope: Option<String>,
}

/// The access token of an OAuth tool server, for the MCP adapter. Core implements it on the
/// stored sign-in; the adapter sends [`AccessTokens::current`] as `Authorization: Bearer` on every
/// request of a connection.
#[async_trait]
pub trait AccessTokens: Send + Sync {
    /// The access token to send, refreshed first when it has expired. Fails with
    /// [`SIGN_IN_REQUIRED`] when the server needs a new sign-in.
    async fn current(&self) -> DomainResult<Secret>;

    /// The server answered `401` to `token`: a fresh token (another request may already have
    /// refreshed it), or [`SIGN_IN_REQUIRED`] once the refresh was rejected, which also marks the
    /// server as needing sign-in.
    async fn rejected(&self, token: &Secret) -> DomainResult<Secret>;
}

/// [`AccessTokens`] in a [`crate::ToolEndpoint`]. Two sources are equal when they are the same
/// object.
#[derive(Clone)]
pub struct TokenSource(pub Arc<dyn AccessTokens>);

impl fmt::Debug for TokenSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TokenSource")
    }
}

impl PartialEq for TokenSource {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for TokenSource {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_secrets() {
        let client = OAuthClient {
            id: "client-1".to_owned(),
            secret: Some(Secret::new("client-secret")),
            auth: ClientAuth::Post,
        };
        let request = TokenRequest {
            endpoint: Url::parse("https://auth.example.com/token").unwrap(),
            client,
            resource: "https://mcp.example.com/mcp".to_owned(),
            grant: Grant::Refresh {
                token: Secret::new("refresh-secret"),
            },
        };
        let tokens = TokenSet {
            access: Secret::new("access-secret"),
            refresh: Some(Secret::new("refresh-secret")),
            expires_in: None,
            scope: None,
        };
        let printed = format!("{request:?} {tokens:?}");
        assert!(!printed.contains("client-secret"), "{printed}");
        assert!(!printed.contains("access-secret"), "{printed}");
        assert!(!printed.contains("refresh-secret"), "{printed}");
        assert!(printed.contains("client-1"), "{printed}");
    }

    #[test]
    fn names_the_client_auth_methods() {
        for method in [ClientAuth::None, ClientAuth::Basic, ClientAuth::Post] {
            assert_eq!(ClientAuth::parse(method.as_str()), Some(method));
        }
        assert_eq!(ClientAuth::parse("private_key_jwt"), None);
    }
}
