//! MCP tool server port.
//!
//! `copilot-mcp` implements [`ToolHost`] over Streamable HTTP with rmcp. Core builds every
//! endpoint, so the adapter never reads the database or the cipher.
//!
//! This step defines tool listing. P1.3 (the conversation engine) adds `open` with `ToolSession`
//! (calls) and `ToolOutput`, and then freezes the port.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

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
}

/// Everything the adapter needs to reach a tool server. `headers` is the opened
/// `ToolHeaders` JSON (read it with `ToolHeaders::entries`); `network` picks the outbound client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolEndpoint {
    pub server_id: ToolServerId,
    pub url: BaseUrl,
    pub headers: Option<Secret>,
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
