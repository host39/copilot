//! [`McpHost`], the `ToolHost` adapter, with its limits and era memory.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use copilot_core::shared::constants::{
    MCP_CLOSE_TIMEOUT_SECS, MCP_CONNECT_TIMEOUT_SECS, MCP_LIST_TIMEOUT_SECS,
    MCP_RESPONSE_MAX_BYTES, MCP_STDIO_START_TIMEOUT_SECS,
};
use copilot_core::{
    DomainResult, Outbound, Target, ToolEndpoint, ToolHost, ToolListing, ToolServerId, ToolSession,
};
use reqwest::Client;
use rmcp::model::ProtocolVersion;

use crate::connection::Connection;
use crate::errors::{Credentials, Failure};
use crate::session::McpSession;

/// Servers whose revision this process remembers; past this the memory starts over.
const ERAS_MAX: usize = 4096;

/// The adapter's time and size limits. [`Limits::default`] holds the production values (core's
/// `MCP_CONNECT_TIMEOUT_SECS`, `MCP_LIST_TIMEOUT_SECS`, `MCP_CLOSE_TIMEOUT_SECS`,
/// `MCP_STDIO_START_TIMEOUT_SECS`, and `MCP_RESPONSE_MAX_BYTES`); tests shorten the times.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// One connection attempt, lifecycle included.
    pub connect: Duration,
    /// A whole `tools/list`, connecting included (a stdio server's start comes on top).
    pub list: Duration,
    /// A stdio server's process: from its start to the end of the MCP handshake. Past it the
    /// process is killed.
    pub stdio_start: Duration,
    /// Closing a connection.
    pub close: Duration,
    /// The largest JSON response or SSE event, and the largest tool list in all
    /// (`MCP_RESPONSE_MAX_BYTES`).
    pub response_max_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(MCP_CONNECT_TIMEOUT_SECS),
            list: Duration::from_secs(MCP_LIST_TIMEOUT_SECS),
            stdio_start: Duration::from_secs(MCP_STDIO_START_TIMEOUT_SECS),
            close: Duration::from_secs(MCP_CLOSE_TIMEOUT_SECS),
            response_max_bytes: MCP_RESPONSE_MAX_BYTES,
        }
    }
}

/// What the host and its sessions share.
pub(crate) struct Shared {
    pub(crate) clients: Outbound<Client>,
    pub(crate) eras: Eras,
    pub(crate) limits: Limits,
    /// `MCP_STDIO_DIR`: where stdio servers' working directories live; `None` refuses them.
    pub(crate) stdio: Option<PathBuf>,
}

/// Implements `ToolHost` over Streamable HTTP with rmcp, on the hardened clients the
/// composition root injects (`copilot_security::clients(Purpose::Tools)`), and over stdio for
/// the programs of stdio servers ([`McpHost::with_stdio`]).
#[derive(Clone)]
pub struct McpHost {
    shared: Arc<Shared>,
}

impl McpHost {
    pub fn new(clients: Outbound<Client>) -> Self {
        Self::with_limits(clients, Limits::default())
    }

    pub fn with_limits(clients: Outbound<Client>, limits: Limits) -> Self {
        Self {
            shared: Arc::new(Shared {
                clients,
                eras: Eras::default(),
                limits,
                stdio: None,
            }),
        }
    }

    /// Runs stdio servers in their directories under `dir` (`MCP_STDIO_DIR`, an absolute path);
    /// without it every stdio server is refused. Whether a server may start at all is core's
    /// gate in its endpoint, asked before every start.
    #[must_use]
    pub fn with_stdio(self, dir: PathBuf) -> Self {
        let shared = &self.shared;
        Self {
            shared: Arc::new(Shared {
                clients: shared.clients.clone(),
                eras: Eras::default(),
                limits: shared.limits,
                stdio: Some(dir),
            }),
        }
    }

    /// The revision this process last negotiated with the server at `url` (`stdio:<command>`
    /// for a stdio server), if any.
    pub fn remembered(&self, server_id: ToolServerId, url: &str) -> Option<String> {
        self.shared
            .eras
            .get(server_id, url)
            .map(|version| version.as_str().to_owned())
    }
}

#[async_trait]
impl ToolHost for McpHost {
    /// Connects, reads every page of `tools/list`, and closes, within `Limits::list` (plus
    /// `Limits::stdio_start` for a stdio server, whose process ends with the listing).
    async fn list(&self, endpoint: &ToolEndpoint) -> DomainResult<ToolListing> {
        let started = Instant::now();
        let shared = &self.shared;
        let limit = match endpoint.target {
            Target::Http(_) => shared.limits.list,
            Target::Stdio(_) => shared.limits.list + shared.limits.stdio_start,
        };
        let listing = async {
            let connection = Connection::open(shared, endpoint).await?;
            let listing = connection.tools().await;
            let version = connection.version().as_str().to_owned();
            connection.close(shared.limits.close).await;
            listing.map(|listing| ToolListing {
                tools: listing.tools,
                protocol_version: version,
                ttl: listing.ttl,
            })
        };
        let listing = tokio::time::timeout(limit, listing).await.map_err(|_| {
            Failure::Timeout.report(endpoint.server_id, Credentials::of(endpoint))
        })??;
        tracing::info!(
            server_id = %endpoint.server_id,
            tools = listing.tools.len(),
            protocol_version = %listing.protocol_version,
            ttl_ms = listing.ttl.map(|ttl| u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX)),
            duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "mcp.tools.listed"
        );
        Ok(listing)
    }

    /// A session that connects to each server on its first call.
    async fn open(&self, endpoints: Vec<ToolEndpoint>) -> DomainResult<Box<dyn ToolSession>> {
        Ok(Box::new(McpSession::new(self.shared.clone(), endpoints)))
    }
}

/// The MCP revision each server spoke last (the spec lets clients cache the era for the life of
/// the origin), keyed by server and URL so one user's server never steers another's.
#[derive(Default)]
pub(crate) struct Eras {
    known: Mutex<HashMap<(ToolServerId, String), ProtocolVersion>>,
}

impl Eras {
    pub(crate) fn get(&self, server_id: ToolServerId, url: &str) -> Option<ProtocolVersion> {
        let known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        known.get(&(server_id, url.to_owned())).cloned()
    }

    pub(crate) fn remember(&self, server_id: ToolServerId, url: &str, version: ProtocolVersion) {
        let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        let key = (server_id, url.to_owned());
        if known.len() >= ERAS_MAX && !known.contains_key(&key) {
            known.clear();
        }
        known.insert(key, version);
    }

    pub(crate) fn forget(&self, server_id: ToolServerId, url: &str) {
        let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        known.remove(&(server_id, url.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_eras_per_server_and_url() {
        let eras = Eras::default();
        let server = ToolServerId::generate();
        let other = ToolServerId::generate();
        eras.remember(
            server,
            "https://a.example/mcp",
            ProtocolVersion::V_2025_11_25,
        );
        assert_eq!(
            eras.get(server, "https://a.example/mcp"),
            Some(ProtocolVersion::V_2025_11_25)
        );
        assert_eq!(eras.get(other, "https://a.example/mcp"), None);
        assert_eq!(eras.get(server, "https://b.example/mcp"), None);
        eras.forget(server, "https://a.example/mcp");
        assert_eq!(eras.get(server, "https://a.example/mcp"), None);
    }

    #[test]
    fn starts_over_when_full() {
        let eras = Eras::default();
        let first = ToolServerId::generate();
        eras.remember(first, "https://a.example", ProtocolVersion::V_2026_07_28);
        for _ in 1..ERAS_MAX {
            eras.remember(
                ToolServerId::generate(),
                "https://a.example",
                ProtocolVersion::V_2026_07_28,
            );
        }
        assert!(eras.get(first, "https://a.example").is_some(), "still fits");
        eras.remember(
            ToolServerId::generate(),
            "https://a.example",
            ProtocolVersion::V_2026_07_28,
        );
        assert_eq!(eras.get(first, "https://a.example"), None);
    }
}
