//! One rmcp client connection to one tool server: the dual-era lifecycle with era memory, the
//! paged tool list with its caps, and tool calls with the unknown-tool refresh.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use copilot_core::shared::constants::{MCP_LIST_MAX_PAGES, MCP_TOOLS_MAX};
use copilot_core::{
    DomainError, DomainResult, RemoteTool, Secret, ToolEndpoint, ToolHeaders, ToolOutput,
    ToolServerId,
};
use reqwest::header::{HeaderName, HeaderValue};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, ErrorCode, ErrorData,
    Implementation, PaginatedRequestParams, ProtocolVersion,
};
use rmcp::service::{ClientInitializeError, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientCacheConfig, ClientLifecycleMode, ClientServiceExt, RoleClient, ServiceError};
use serde_json::{Map, Value};

use crate::convert;
use crate::errors::{self, Credentials, Failure};
use crate::host::Shared;

/// The modern revision: stateless, no `initialize`, `server/discover` up front.
const MODERN: ProtocolVersion = ProtocolVersion::V_2026_07_28;
/// The newest revision with the `initialize` handshake and `Mcp-Session-Id` sessions.
const LEGACY: ProtocolVersion = ProtocolVersion::LATEST_WITH_INITIALIZE;

type Client = RunningService<RoleClient, ClientConfig>;

/// How a connection starts.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Lifecycle {
    /// `server/discover` first, then `initialize` when the server turns out to be legacy.
    Auto,
    /// `server/discover` only, for a server remembered as modern.
    Modern(ProtocolVersion),
    /// `initialize` only, for a server remembered as legacy: no modern probe (which a silent
    /// legacy server answers only after rmcp's 10-second wait).
    Legacy,
}

impl Lifecycle {
    fn remembered(version: Option<ProtocolVersion>) -> Self {
        match version {
            None => Self::Auto,
            Some(version) if version.has_initialize() => Self::Legacy,
            Some(version) => Self::Modern(version),
        }
    }

    fn mode(&self) -> ClientLifecycleMode {
        match self {
            Self::Auto => ClientLifecycleMode::Auto {
                preferred_versions: vec![MODERN],
                legacy_version: Some(LEGACY),
            },
            Self::Modern(version) => ClientLifecycleMode::Discover {
                preferred_versions: vec![version.clone()],
            },
            Self::Legacy => ClientLifecycleMode::Initialize,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Modern(_) => "modern",
            Self::Legacy => "legacy",
        }
    }
}

/// Why one connection attempt failed.
enum Attempt {
    Failed(Box<ClientInitializeError>),
    Elapsed,
}

impl Attempt {
    fn failure(&self) -> Failure {
        match self {
            Self::Failed(error) => Failure::connect(error),
            Self::Elapsed => Failure::Timeout,
        }
    }

    /// The server's versions, when it supports none of the client's modern ones but does speak
    /// a revision with `initialize`.
    fn legacy_offered(&self) -> bool {
        matches!(
            self,
            Self::Failed(error) if matches!(
                error.as_ref(),
                ClientInitializeError::NoCompatibleProtocolVersion { server_supported, .. }
                    if server_supported.iter().any(ProtocolVersion::has_initialize)
            )
        )
    }
}

/// Why a call on an open connection failed.
pub(crate) enum CallError {
    /// The server answered `401` to the connection's access token: refresh it and reconnect.
    Unauthorized,
    Failed(DomainError),
}

/// An open connection to one server.
pub(crate) struct Connection {
    client: Client,
    server_id: ToolServerId,
    version: ProtocolVersion,
    /// What requests carry, for the sign-in message.
    credentials: Credentials,
    /// The access token every request carries, for a server signed in with OAuth.
    token: Option<Secret>,
    /// `MCP_RESPONSE_MAX_BYTES`: also the cap on a whole tool list.
    max_bytes: usize,
}

impl Connection {
    /// Connects with the endpoint's credentials. A server signed in with OAuth gets the current
    /// access token on every request; when it answers `401`, the token is refreshed once
    /// (`AccessTokens::rejected`) and the connection tried again.
    pub(crate) async fn open(shared: &Shared, endpoint: &ToolEndpoint) -> DomainResult<Self> {
        let credentials = Credentials::of(endpoint);
        let report = |failure: Failure| failure.report(endpoint.server_id, credentials);
        let Some(tokens) = &endpoint.tokens else {
            return Self::connect(shared, endpoint, None).await.map_err(report);
        };
        let token = tokens.0.current().await?;
        match Self::connect(shared, endpoint, Some(token.clone())).await {
            Err(failure) if failure.unauthorized() => {
                tracing::info!(server_id = %endpoint.server_id, "mcp.oauth.token-rejected");
                let fresh = tokens.0.rejected(&token).await?;
                Self::connect(shared, endpoint, Some(fresh))
                    .await
                    .map_err(report)
            }
            other => other.map_err(report),
        }
    }

    /// Connects with `token` as the access token, without another refresh.
    pub(crate) async fn with_token(
        shared: &Shared,
        endpoint: &ToolEndpoint,
        token: Secret,
    ) -> DomainResult<Self> {
        Self::connect(shared, endpoint, Some(token))
            .await
            .map_err(|failure| failure.report(endpoint.server_id, Credentials::SignIn))
    }

    /// Connects with the revision remembered for the server (this process's memory first, then
    /// the endpoint's stored `protocol_version`), or probes both eras when none is known. A
    /// remembered revision that the server no longer speaks falls back to the probe, and a probe
    /// that finds only legacy revisions behind `server/discover` falls back to `initialize`.
    async fn connect(
        shared: &Shared,
        endpoint: &ToolEndpoint,
        token: Option<Secret>,
    ) -> Result<Self, Failure> {
        let server_id = endpoint.server_id;
        let credentials = Credentials::of(endpoint);
        let http = shared
            .clients
            .pick(endpoint.url.url(), endpoint.network)
            .map_err(Failure::Blocked)?;
        let headers = headers(endpoint, token.as_ref())?;
        let url = endpoint.url.url().as_str();
        let remembered = shared
            .eras
            .get(server_id, url)
            .or_else(|| endpoint.protocol_version.as_deref().and_then(known_version));
        let first = Lifecycle::remembered(remembered);
        let attempt = |lifecycle: Lifecycle| {
            let config = StreamableHttpClientTransportConfig::with_uri(url)
                .custom_headers(headers.clone())
                .max_json_response_size(shared.limits.response_max_bytes)
                .max_sse_event_size(shared.limits.response_max_bytes);
            let transport = StreamableHttpClientTransport::with_client(http.clone(), config);
            let info = ClientConfig::new(
                ClientCapabilities::default(),
                Implementation::new("copilot", env!("CARGO_PKG_VERSION")),
            );
            let info = match lifecycle {
                Lifecycle::Legacy => info.with_protocol_version(LEGACY),
                _ => info,
            };
            let connect = shared.limits.connect;
            async move {
                match tokio::time::timeout(
                    connect,
                    info.serve_with_lifecycle(transport, lifecycle.mode()),
                )
                .await
                {
                    Ok(Ok(client)) => Ok(client),
                    Ok(Err(error)) => Err(Attempt::Failed(Box::new(error))),
                    Err(_) => Err(Attempt::Elapsed),
                }
            }
        };

        let mut result = attempt(first.clone()).await;
        if let Err(failed) = &result
            && first != Lifecycle::Auto
            && failed.failure().era_mismatch()
        {
            tracing::info!(
                server_id = %server_id,
                remembered = first.name(),
                "mcp.connect.era-changed"
            );
            result = attempt(Lifecycle::Auto).await;
        }
        if let Err(failed) = &result
            && first != Lifecycle::Legacy
            && failed.legacy_offered()
        {
            result = attempt(Lifecycle::Legacy).await;
        }
        let client = match result {
            Ok(client) => client,
            Err(failed) => {
                shared.eras.forget(server_id, url);
                return Err(failed.failure());
            }
        };
        // Core keeps the tool list; rmcp's own response cache would serve stale lists to the
        // unknown-tool refresh.
        client
            .set_response_cache_config(ClientCacheConfig::disabled())
            .await;
        let version = client
            .peer_info()
            .map(|info| info.protocol_version.clone())
            .unwrap_or(LEGACY);
        shared.eras.remember(server_id, url, version.clone());
        tracing::debug!(
            server_id = %server_id,
            protocol_version = %version,
            "mcp.connect.opened"
        );
        Ok(Self {
            client,
            server_id,
            version,
            credentials,
            token,
            max_bytes: shared.limits.response_max_bytes,
        })
    }

    /// The access token the connection's requests carry.
    pub(crate) fn token(&self) -> Option<&Secret> {
        self.token.as_ref()
    }

    /// The negotiated MCP revision.
    pub(crate) fn version(&self) -> &ProtocolVersion {
        &self.version
    }

    /// Every page of `tools/list`, at most `MCP_LIST_MAX_PAGES` pages, `MCP_TOOLS_MAX` tools,
    /// and `MCP_RESPONSE_MAX_BYTES` of tool definitions in all. The cache hint is the smallest `ttlMs` of the pages, or
    /// `None` when a page has none.
    pub(crate) async fn tools(&self) -> DomainResult<Listing> {
        let mut tools = Vec::new();
        let mut bytes = 0;
        let mut ttl_ms = Some(u64::MAX);
        let mut cursor = None;
        for _ in 0..MCP_LIST_MAX_PAGES {
            let params = PaginatedRequestParams::default().with_cursor(cursor);
            let page = self
                .client
                .list_tools(Some(params))
                .await
                .map_err(|error| self.list_error(&error))?;
            if tools.len() + page.tools.len() > MCP_TOOLS_MAX {
                tracing::warn!(server_id = %self.server_id, "mcp.tools.list-too-many");
                return Err(errors::too_many());
            }
            for tool in page.tools {
                bytes += serde_json::to_vec(&tool).map_or(0, |encoded| encoded.len());
                if bytes > self.max_bytes {
                    tracing::warn!(server_id = %self.server_id, bytes, "mcp.tools.list-too-large");
                    return Err(errors::too_large());
                }
                tools.push(convert::tool(tool));
            }
            ttl_ms = ttl_ms.zip(page.ttl_ms).map(|(left, right)| left.min(right));
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(Listing {
                    tools,
                    ttl: ttl_ms.map(Duration::from_millis),
                });
            }
        }
        tracing::warn!(server_id = %self.server_id, "mcp.tools.list-too-long");
        Err(errors::too_large())
    }

    /// Calls `name`. When the server answers that the tool is unknown or its headers do not
    /// match (`-32602` or `-32020`), the tools are listed again on this connection, which also
    /// gives rmcp the input schemas it needs for `Mcp-Param-*` headers. A tool that is still
    /// listed is called once more after a header mismatch; one that is gone fails with
    /// `TOOL_UNKNOWN`. A `401` to the connection's access token is [`CallError::Unauthorized`].
    pub(crate) async fn call(
        &self,
        name: &str,
        arguments: &Value,
    ) -> Result<ToolOutput, CallError> {
        let started = Instant::now();
        let result = match self.send(name, arguments).await {
            Err(ServiceError::McpError(data))
                if data.code == ErrorCode::INVALID_PARAMS
                    || data.code == ErrorCode::HEADER_MISMATCH =>
            {
                self.refresh(name, arguments, data)
                    .await
                    .map_err(CallError::Failed)?
            }
            other => other,
        };
        if let Err(error) = &result
            && self.token.is_some()
            && Failure::request(error).unauthorized()
        {
            return Err(CallError::Unauthorized);
        }
        let output = self.finish(name, result).map_err(CallError::Failed)?;
        tracing::info!(
            server_id = %self.server_id,
            tool = name,
            is_error = output.is_error,
            parts = output.content.len(),
            duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "mcp.tools.called"
        );
        Ok(output)
    }

    /// Lists the tools again after `data` (unknown tool or header mismatch). Fails with
    /// `TOOL_UNKNOWN` when `name` is gone; calls it once more after a header mismatch.
    async fn refresh(
        &self,
        name: &str,
        arguments: &Value,
        data: ErrorData,
    ) -> DomainResult<Result<CallToolResult, ServiceError>> {
        let listed = self.names().await?.contains(name);
        tracing::info!(
            server_id = %self.server_id,
            tool = name,
            code = data.code.0,
            listed,
            "mcp.tools.refreshed"
        );
        if !listed {
            return Err(errors::unknown_tool());
        }
        Ok(if data.code == ErrorCode::HEADER_MISMATCH {
            self.send(name, arguments).await
        } else {
            Err(ServiceError::McpError(data))
        })
    }

    /// The call's output, or its failure as a readable error.
    fn finish(
        &self,
        name: &str,
        result: Result<CallToolResult, ServiceError>,
    ) -> DomainResult<ToolOutput> {
        match result {
            Ok(result) => Ok(convert::output(result)),
            Err(ServiceError::McpError(data)) => {
                tracing::info!(
                    server_id = %self.server_id,
                    tool = name,
                    code = data.code.0,
                    "mcp.tools.call-refused"
                );
                Err(errors::refused(&data))
            }
            Err(error) => Err(Failure::request(&error).report(self.server_id, self.credentials)),
        }
    }

    /// Whether the connection can take no more requests (the transport is gone).
    pub(crate) fn is_closed(&self) -> bool {
        self.client.is_transport_closed()
    }

    /// Ends the connection (a legacy session also sends its `DELETE`), waiting at most `limit`.
    pub(crate) async fn close(mut self, limit: Duration) {
        if let Err(error) = self.client.close_with_timeout(limit).await {
            tracing::debug!(server_id = %self.server_id, error = %error, "mcp.connect.close-failed");
        }
    }

    async fn send(&self, name: &str, arguments: &Value) -> Result<CallToolResult, ServiceError> {
        let arguments: Map<String, Value> = match arguments {
            Value::Object(object) => object.clone(),
            _ => Map::new(),
        };
        let params = CallToolRequestParams::new(name.to_owned()).with_arguments(arguments);
        self.client.call_tool(params).await
    }

    /// The names the server lists now.
    async fn names(&self) -> DomainResult<HashSet<String>> {
        let listing = self.tools().await?;
        Ok(listing.tools.into_iter().map(|tool| tool.name).collect())
    }

    fn list_error(&self, error: &ServiceError) -> DomainError {
        match error {
            ServiceError::McpError(data) => errors::list_refused(self.server_id, data),
            error => Failure::request(error).report(self.server_id, self.credentials),
        }
    }
}

/// A server's tools with the cache hint.
pub(crate) struct Listing {
    pub(crate) tools: Vec<RemoteTool>,
    pub(crate) ttl: Option<Duration>,
}

/// The endpoint's stored credentials, or the access token as `Authorization: Bearer`, as
/// headers, every value marked sensitive.
fn headers(
    endpoint: &ToolEndpoint,
    token: Option<&Secret>,
) -> Result<HashMap<HeaderName, HeaderValue>, Failure> {
    let entries = match (token, &endpoint.headers) {
        (Some(token), _) => {
            let token = token.text().ok_or(Failure::Headers)?;
            vec![("Authorization".to_owned(), format!("Bearer {token}"))]
        }
        (None, Some(secret)) => ToolHeaders::entries(secret).map_err(|_| Failure::Headers)?,
        (None, None) => return Ok(HashMap::new()),
    };
    entries
        .into_iter()
        .map(|(name, value)| {
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| Failure::Headers)?;
            let mut value = HeaderValue::from_str(&value).map_err(|_| Failure::Headers)?;
            value.set_sensitive(true);
            Ok((name, value))
        })
        .collect()
}

/// A stored revision rmcp knows; anything else is ignored and the eras are probed again.
fn known_version(text: &str) -> Option<ProtocolVersion> {
    ProtocolVersion::KNOWN_VERSIONS
        .iter()
        .find(|version| version.as_str() == text)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_lifecycle_from_the_remembered_revision() {
        assert_eq!(Lifecycle::remembered(None), Lifecycle::Auto);
        assert_eq!(
            Lifecycle::remembered(Some(ProtocolVersion::V_2025_11_25)),
            Lifecycle::Legacy
        );
        assert_eq!(
            Lifecycle::remembered(Some(ProtocolVersion::V_2025_03_26)),
            Lifecycle::Legacy
        );
        assert_eq!(
            Lifecycle::remembered(Some(ProtocolVersion::V_2026_07_28)),
            Lifecycle::Modern(ProtocolVersion::V_2026_07_28)
        );
    }

    #[test]
    fn ignores_a_stored_revision_it_does_not_know() {
        assert_eq!(
            known_version("2025-11-25"),
            Some(ProtocolVersion::V_2025_11_25)
        );
        assert_eq!(known_version("2027-01-01"), None);
        assert_eq!(known_version(""), None);
    }
}
