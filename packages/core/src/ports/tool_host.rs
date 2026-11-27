//! MCP tool server port.
//!
//! `copilot-mcp` implements [`ToolHost`] over Streamable HTTP with rmcp. Core builds every
//! endpoint, so the adapter never reads the database or the cipher.
//!
//! The port is frozen from P1.3 on (`docs/design/roadmap.md`, Freeze List).

use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use serde_json::Value;

use crate::ports::tool_auth::TokenSource;
use crate::shared::base_url::BaseUrl;
use crate::shared::errors::DomainResult;
use crate::shared::ids::ToolServerId;
use crate::shared::net::NetworkScope;
use crate::shared::secret::Secret;

#[async_trait]
pub trait ToolHost: Send + Sync {
    /// The server's tools (`tools/list`, every page). Adapters map connection failures to
    /// human-readable `DomainError`s.
    async fn list(&self, endpoint: &ToolEndpoint) -> DomainResult<ToolListing>;

    /// A session for one run over `endpoints`. The adapter connects to each server lazily, on its
    /// first call, and keeps one connection per server until [`ToolSession::close`].
    async fn open(&self, endpoints: Vec<ToolEndpoint>) -> DomainResult<Box<dyn ToolSession>>;
}

/// The error code of a call that ran out of time; core gives the call the status `timeout`.
pub const TOOL_TIMEOUT: &str = "TOOL_TIMEOUT";
/// The error code of a call to a tool the server no longer lists, even after listing again;
/// core marks the server's stored tool list stale, so the next turn lists it again.
pub const TOOL_UNKNOWN: &str = "TOOL_UNKNOWN";

/// The tool calls of one run.
#[async_trait]
pub trait ToolSession: Send + Sync {
    /// Calls tool `name` (the original name, never the alias) on the server. A call that does
    /// not answer within `timeout` fails with the code [`TOOL_TIMEOUT`], a tool the server no
    /// longer lists with [`TOOL_UNKNOWN`]; other failures are human-readable `DomainError`s. A tool that ran and reported an error is `Ok` with
    /// `is_error`.
    async fn call(
        &self,
        server_id: ToolServerId,
        name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> DomainResult<ToolOutput>;

    /// Ends every connection of the session.
    async fn close(self: Box<Self>);
}

/// What a tool returned, untruncated (core truncates text to `MCP_RESULT_MAX_BYTES`).
#[derive(Clone, Debug, PartialEq)]
pub struct ToolOutput {
    pub content: Vec<ToolContent>,
    pub is_error: bool,
}

/// A piece of tool output. Everything here is untrusted server input.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolContent {
    Text(String),
    Image {
        media_type: String,
        data: Bytes,
    },
    Audio {
        media_type: String,
        data: Bytes,
    },
    /// A `resource_link` (or an embedded resource without text): sent to the model as text.
    Link {
        uri: String,
        name: Option<String>,
    },
    /// Any other binary content: sent to the model as a one-line notice.
    Binary {
        media_type: Option<String>,
    },
}

/// Everything the adapter needs to reach a tool server. `headers` is the opened
/// `ToolHeaders` JSON (read it with `ToolHeaders::entries`); `tokens`, for a server signed in with
/// OAuth, gives the access token to send as `Authorization: Bearer` and a fresh one after a `401`
/// (a server has one or the other, never both); `network` picks the outbound client, and a plain
/// `http` `url` (`!is_https()`) takes the plaintext client, whose resolver admits only
/// `net::is_allowed_plaintext` addresses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolEndpoint {
    pub server_id: ToolServerId,
    pub url: BaseUrl,
    pub headers: Option<Secret>,
    pub tokens: Option<TokenSource>,
    /// The MCP revision negotiated last time, so a legacy server skips the modern probe.
    pub protocol_version: Option<String>,
    pub network: NetworkScope,
}

/// A `tools/list` result.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolListing {
    pub tools: Vec<RemoteTool>,
    /// The negotiated MCP revision.
    pub protocol_version: String,
    /// The server's `ttlMs`; `None` uses `TOOL_CACHE_DEFAULT_TTL_SECS`.
    pub ttl: Option<Duration>,
}

/// A tool as the server lists it. Everything here is untrusted server input.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteTool {
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    pub input_schema: Value,
    /// The `readOnlyHint` annotation. A hint only: it suggests `auto` in the UI and grants
    /// nothing.
    pub read_only: bool,
}
