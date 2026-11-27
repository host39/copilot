//! An in-process MCP server for the adapter tests: rmcp's own Streamable HTTP server, served by
//! axum on loopback, in either era. A middleware in front of it records each JSON-RPC method and
//! the credential headers, and can demand a bearer token.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData,
    ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig,
    Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, ServerHandler};
use serde_json::{Map, Value, json};

/// Tools per `tools/list` page, so a listing of the default tools takes several pages.
const PAGE_SIZE: usize = 3;

/// Which MCP the server speaks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Era {
    /// 2026-07-28 (and every older revision): stateless, `server/discover`.
    Modern,
    /// Only 2026-07-28: `initialize` fails.
    ModernOnly,
    /// Up to 2025-11-25 with `Mcp-Session-Id` sessions; `server/discover` gets a plain-text
    /// `400`, as from a server written before the method existed.
    Legacy,
    /// Up to 2025-11-25 with sessions, but answering `server/discover` with its legacy
    /// revisions (a current SDK configured for old clients only).
    LegacyDiscover,
}

/// What the server saw.
#[derive(Default)]
pub struct Seen {
    /// Every HTTP request (POSTs, a legacy session's GET stream and DELETE).
    pub requests: usize,
    /// JSON-RPC methods in arrival order.
    pub methods: Vec<String>,
    /// `Authorization` values.
    pub authorization: Vec<String>,
    /// `X-Team` values.
    pub team: Vec<String>,
    /// Requests that carried `Mcp-Session-Id`.
    pub sessions: usize,
}

impl Seen {
    pub fn count(&self, method: &str) -> usize {
        self.methods.iter().filter(|seen| *seen == method).count()
    }
}

#[derive(Clone)]
pub struct TestServer {
    era: Era,
    tools: Arc<Mutex<Vec<Tool>>>,
    ttl_ms: Option<u64>,
    /// Set when the `sleep` tool saw its request cancelled.
    pub cancelled: Arc<AtomicBool>,
}

/// A running server.
pub struct Running {
    pub url: String,
    pub seen: Arc<Mutex<Seen>>,
    pub server: TestServer,
}

impl Running {
    pub fn seen(&self) -> std::sync::MutexGuard<'_, Seen> {
        self.seen.lock().unwrap()
    }

    /// Replaces the tool list (as a server update would).
    pub fn set_tools(&self, tools: Vec<Tool>) {
        *self.server.tools.lock().unwrap() = tools;
    }
}

/// Options of [`start`].
#[derive(Clone, Default)]
pub struct Options {
    /// Answer `401` unless `Authorization` is exactly this.
    pub require: Option<String>,
    /// `ttlMs` on every page; the last page halves it, so the merged hint is the smaller one.
    pub ttl_ms: Option<u64>,
    /// JSON responses instead of SSE streams.
    pub json: bool,
}

pub async fn start(era: Era, options: Options) -> Running {
    let server = TestServer {
        era,
        tools: Arc::new(Mutex::new(default_tools())),
        ttl_ms: options.ttl_ms,
        cancelled: Arc::new(AtomicBool::new(false)),
    };
    let legacy = matches!(era, Era::Legacy | Era::LegacyDiscover);
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(legacy)
        .with_json_response(options.json)
        .with_sse_keep_alive(None);
    let factory = server.clone();
    let service: StreamableHttpService<TestServer, LocalSessionManager> =
        StreamableHttpService::new(move || Ok(factory.clone()), Default::default(), config);
    let seen = Arc::new(Mutex::new(Seen::default()));
    let guard = Guard {
        seen: seen.clone(),
        require: options.require,
        era,
    };
    let router = axum::Router::new()
        .nest_service("/mcp", service)
        .layer(from_fn_with_state(guard, record));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Running {
        url: format!("http://{address}/mcp"),
        seen,
        server,
    }
}

#[derive(Clone)]
struct Guard {
    seen: Arc<Mutex<Seen>>,
    require: Option<String>,
    era: Era,
}

/// Records the request, then refuses it (wrong bearer, or `server/discover` on a [`Era::Legacy`]
/// server) or passes it on.
async fn record(State(guard): State<Guard>, request: Request, next: Next) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, usize::MAX).await.unwrap();
    let method = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|value| value.get("method")?.as_str().map(str::to_owned));
    let header = |name: &str| {
        parts
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let authorization = header("authorization");
    {
        let mut seen = guard.seen.lock().unwrap();
        seen.requests += 1;
        if let Some(method) = &method {
            seen.methods.push(method.clone());
        }
        seen.authorization.extend(authorization.clone());
        seen.team.extend(header("x-team"));
        if parts.headers.contains_key("mcp-session-id") {
            seen.sessions += 1;
        }
    }
    if let Some(required) = &guard.require
        && authorization.as_deref() != Some(required.as_str())
    {
        return (
            StatusCode::UNAUTHORIZED,
            [("www-authenticate", "Bearer")],
            "unauthorized",
        )
            .into_response();
    }
    if guard.era == Era::Legacy && method.as_deref() == Some("server/discover") {
        return (
            StatusCode::BAD_REQUEST,
            "Bad Request: No valid session ID provided",
        )
            .into_response();
    }
    next.run(Request::from_parts(parts, Body::from(bytes)))
        .await
}

fn schema(properties: Value) -> Map<String, Value> {
    let Value::Object(object) = json!({"type": "object", "properties": properties}) else {
        unreachable!("an object literal");
    };
    object
}

pub fn tool(name: &'static str, description: &'static str, properties: Value) -> Tool {
    Tool::new(name, description, schema(properties))
}

/// `echo`, `region` (its argument mirrored in the `Mcp-Param-Region` header), `sleep`, `big`,
/// `picture`, and `fail`.
pub fn default_tools() -> Vec<Tool> {
    let mut echo = tool("echo", "Echoes text.", json!({"text": {"type": "string"}}));
    echo.annotations = Some(ToolAnnotations::with_title("Echo").read_only(true));
    vec![
        echo,
        tool(
            "region",
            "Names the region.",
            json!({"region": {"type": "string", "x-mcp-header": "Region"}}),
        ),
        tool("sleep", "Waits.", json!({"ms": {"type": "integer"}})),
        tool(
            "big",
            "Sends many bytes.",
            json!({"bytes": {"type": "integer"}}),
        ),
        tool("picture", "Sends an image.", json!({})),
        tool("fail", "Reports an error.", json!({})),
    ]
}

/// A one-pixel PNG.
pub const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn argument<'a>(request: &'a CallToolRequestParams, name: &str) -> Option<&'a Value> {
    request.arguments.as_ref()?.get(name)
}

impl ServerHandler for TestServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        match self.era {
            Era::Modern => Cow::Borrowed(ProtocolVersion::KNOWN_VERSIONS),
            Era::ModernOnly => Cow::Owned(vec![ProtocolVersion::V_2026_07_28]),
            Era::Legacy | Era::LegacyDiscover => {
                Cow::Borrowed(ProtocolVersion::known_up_to(&ProtocolVersion::V_2025_11_25))
            }
        }
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        let tools = self.tools.lock().unwrap();
        tools.iter().find(|tool| tool.name == name).cloned()
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = self.tools.lock().unwrap().clone();
        let start: usize = request
            .and_then(|request| request.cursor)
            .map_or(0, |cursor| cursor.parse().unwrap());
        let end = (start + PAGE_SIZE).min(tools.len());
        let last = end == tools.len();
        let mut page = ListToolsResult::with_all_items(tools[start..end].to_vec());
        page.next_cursor = (!last).then(|| end.to_string());
        page.ttl_ms = self
            .ttl_ms
            .map(|ttl| if last && start > 0 { ttl / 2 } else { ttl });
        Ok(page)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if self.get_tool(&request.name).is_none() {
            return Err(ErrorData::invalid_params(
                format!("Unknown tool: {}", request.name),
                None,
            ));
        }
        let result = match request.name.as_ref() {
            "echo" => {
                let text = argument(&request, "text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                CallToolResult::success(vec![ContentBlock::text(format!("echo: {text}"))])
            }
            "region" => {
                let region = argument(&request, "region")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                CallToolResult::success(vec![ContentBlock::text(format!("region: {region}"))])
            }
            "sleep" => {
                let ms = argument(&request, "ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_millis(ms)) => {}
                    () = context.ct.cancelled() => {
                        self.cancelled.store(true, Ordering::SeqCst);
                        return Err(ErrorData::internal_error("cancelled", None));
                    }
                }
                CallToolResult::success(vec![ContentBlock::text("done")])
            }
            "big" => {
                let bytes = argument(&request, "bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let bytes = usize::try_from(bytes).unwrap();
                CallToolResult::success(vec![ContentBlock::text("x".repeat(bytes))])
            }
            "picture" => {
                use base64::Engine;
                let data = base64::engine::general_purpose::STANDARD.encode(PNG);
                CallToolResult::success(vec![
                    ContentBlock::image(data, "image/png"),
                    ContentBlock::text("a pixel"),
                ])
            }
            "fail" => CallToolResult::error(vec![ContentBlock::text("the disk is full")]),
            // Only listed when a test adds it: sends its `data` argument as a log notification
            // during the call, as untrusted server text the client receives. (MCP deprecates
            // logging, but servers still send it.)
            "notify" => {
                let data = argument(&request, "data").cloned().unwrap_or(Value::Null);
                #[allow(deprecated)]
                let notification = rmcp::model::LoggingMessageNotificationParam::new(
                    rmcp::model::LoggingLevel::Info,
                    data,
                );
                #[allow(deprecated)]
                let sent = context.peer.notify_logging_message(notification).await;
                sent.map_err(|_| ErrorData::internal_error("notification failed", None))?;
                CallToolResult::success(vec![ContentBlock::text("notified")])
            }
            other => CallToolResult::success(vec![ContentBlock::text(format!("{other} ran"))]),
        };
        Ok(result.into())
    }
}
