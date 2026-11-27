//! [`McpAuth`], the `ToolAuth` adapter: the OAuth requests of a tool server sign-in (MCP
//! authorization, revision `2026-07-28`), hand-written on the hardened clients rather than
//! through rmcp's `auth` feature (`docs/design/tracks/p7-2.md`, Decisions).
//!
//! Rules:
//!
//! - Every request picks its client with `Outbound::pick` for its own URL and the account's
//!   network scope, so metadata, registration, and token URLs that a server hands back follow the
//!   same SSRF rules and private-network policy as the server's own URL. No redirects are
//!   followed, every request has `OAUTH_TIMEOUT_SECS`, and every answer is capped at
//!   `OAUTH_RESPONSE_MAX_BYTES`.
//! - Logs carry step names, statuses, and failure kinds (`mcp.oauth.*`); never a URL, a code, a
//!   token, a secret, or a server's error description.

mod challenge;
mod discovery;
mod http;
mod token;

use async_trait::async_trait;
use copilot_core::{
    AuthServer, BaseUrl, Discovery, DomainError, DomainResult, NetworkScope, OAUTH_BLOCKED,
    OAUTH_FAILED, OAuthClient, Outbound, Registration, TokenRequest, TokenSet, ToolAuth,
};
use reqwest::Client;

use crate::oauth::http::{Http, Trouble};

/// Implements `ToolAuth` on the hardened clients the composition root injects
/// (`copilot_security::clients(Purpose::Tools)`, the same ones `McpHost` uses).
#[derive(Clone)]
pub struct McpAuth {
    clients: Outbound<Client>,
}

impl McpAuth {
    pub fn new(clients: Outbound<Client>) -> Self {
        Self { clients }
    }

    fn http(&self, network: NetworkScope) -> Http<'_> {
        Http {
            clients: &self.clients,
            network,
        }
    }
}

#[async_trait]
impl ToolAuth for McpAuth {
    async fn discover(&self, url: &BaseUrl, network: NetworkScope) -> DomainResult<Discovery> {
        discovery::discover(&self.http(network), url).await
    }

    async fn register(
        &self,
        server: &AuthServer,
        registration: &Registration,
        network: NetworkScope,
    ) -> DomainResult<OAuthClient> {
        token::register(&self.http(network), server, registration).await
    }

    async fn token(&self, request: &TokenRequest, network: NetworkScope) -> DomainResult<TokenSet> {
        token::token(&self.http(network), request).await
    }
}

/// A request of `step` that got no answer: logged (`mcp.oauth.blocked` or `mcp.oauth.failed`)
/// and told to the user without the address.
pub(crate) fn trouble(step: &'static str, trouble: Trouble) -> DomainError {
    match trouble {
        Trouble::Blocked(reason) => {
            tracing::warn!(step, reason = %reason, "mcp.oauth.blocked");
            DomainError::upstream(
                "This account may not reach the sign-in addresses of this tool server.",
                OAUTH_BLOCKED,
            )
        }
        Trouble::Timeout => {
            tracing::warn!(step, failure = trouble.kind(), "mcp.oauth.failed");
            DomainError::upstream(
                "The sign-in server did not answer in time. Try again later.",
                OAUTH_FAILED,
            )
        }
        other => {
            tracing::warn!(step, failure = other.kind(), "mcp.oauth.failed");
            DomainError::upstream(
                "Could not reach the sign-in server. Try again later.",
                OAUTH_FAILED,
            )
        }
    }
}
