//! An in-process MCP tool server for route tests: rmcp's own Streamable HTTP server on loopback
//! (`http://127.0.0.1:<port>/mcp`, which only a private-scope connection may reach), in either
//! era, with a tool list the test can change and a bearer token it can demand (and change, with
//! the `WWW-Authenticate` challenge its `401` carries, for the sign-in tests).

use std::borrow::Cow;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData,
    ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig,
    Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, ServerHandler};
use serde_json::{Map, Value};

pub struct McpServer {
    pub url: String,
    tools: Arc<Mutex<Vec<Tool>>>,
    gate: Arc<Gate>,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
}

impl McpServer {
    /// A server that speaks 2026-07-28 (`legacy = false`) or only 2025-11-25 and older with
    /// `Mcp-Session-Id` sessions, listing `tools` (name, description, input schema), and
    /// answering `401` unless `Authorization` is `bearer` when one is given.
    pub async fn start(
        legacy: bool,
        tools: &[(String, String, Value)],
        bearer: Option<&str>,
    ) -> Self {
        let handler = Handler {
            tools: Arc::new(Mutex::new(Vec::new())),
            calls: Arc::new(Mutex::new(Vec::new())),
            legacy,
        };
        let config = StreamableHttpServerConfig::default()
            .with_legacy_session_mode(legacy)
            .with_sse_keep_alive(None);
        let factory = handler.clone();
        let service: StreamableHttpService<Handler, LocalSessionManager> =
            StreamableHttpService::new(move || Ok(factory.clone()), Default::default(), config);
        let gate = Arc::new(Gate {
            required: Mutex::new(bearer.map(|token| format!("Bearer {token}"))),
            challenge: Mutex::new("Bearer".to_owned()),
            requests: AtomicUsize::new(0),
            authorization: Mutex::new(Vec::new()),
        });
        let router = axum::Router::new()
            .nest_service("/mcp", service)
            .layer(from_fn_with_state(gate.clone(), guard));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port");
        let address = listener.local_addr().expect("an address");
        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("the MCP server runs");
        });
        let server = Self {
            url: format!("http://{address}/mcp"),
            tools: handler.tools,
            gate,
            calls: handler.calls,
        };
        server.set_tools(tools);
        server
    }

    /// Replaces the tool list, as a server update would.
    pub fn set_tools(&self, tools: &[(String, String, Value)]) {
        let tools = tools
            .iter()
            .map(|(name, description, schema)| {
                let schema = match schema {
                    Value::Object(object) => object.clone(),
                    _ => Map::new(),
                };
                Tool::new(name.clone(), description.clone(), schema)
            })
            .collect();
        *self.tools.lock().expect("tools") = tools;
    }

    /// HTTP requests that reached the server.
    pub fn requests(&self) -> usize {
        self.gate.requests.load(Ordering::SeqCst)
    }

    /// From now on, answers `401` unless `Authorization` is `Bearer <token>`.
    pub fn require(&self, token: &str) {
        *self.gate.required.lock().expect("required") = Some(format!("Bearer {token}"));
    }

    /// The `WWW-Authenticate` value of its `401`s.
    pub fn challenge(&self, value: &str) {
        *self.gate.challenge.lock().expect("challenge") = value.to_owned();
    }

    /// Every `Authorization` value the server saw, in order.
    pub fn authorization(&self) -> Vec<String> {
        self.gate
            .authorization
            .lock()
            .expect("authorization")
            .clone()
    }

    /// The tool calls the server answered, in order: each tool's name and arguments.
    pub fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().expect("calls").clone()
    }
}

struct Gate {
    required: Mutex<Option<String>>,
    challenge: Mutex<String>,
    requests: AtomicUsize,
    authorization: Mutex<Vec<String>>,
}

async fn guard(State(gate): State<Arc<Gate>>, request: Request, next: Next) -> Response {
    gate.requests.fetch_add(1, Ordering::SeqCst);
    let given = request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    if let Some(given) = &given {
        gate.authorization
            .lock()
            .expect("authorization")
            .push(given.clone());
    }
    let required = gate.required.lock().expect("required").clone();
    if let Some(required) = required
        && given.as_deref() != Some(required.as_str())
    {
        let challenge = gate.challenge.lock().expect("challenge").clone();
        return (StatusCode::UNAUTHORIZED, [("www-authenticate", challenge)]).into_response();
    }
    next.run(request).await
}

#[derive(Clone)]
struct Handler {
    tools: Arc<Mutex<Vec<Tool>>>,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
    legacy: bool,
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        if self.legacy {
            Cow::Borrowed(ProtocolVersion::known_up_to(&ProtocolVersion::V_2025_11_25))
        } else {
            Cow::Borrowed(ProtocolVersion::KNOWN_VERSIONS)
        }
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        let tools = self.tools.lock().expect("tools");
        tools.iter().find(|tool| tool.name == name).cloned()
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = self.tools.lock().expect("tools").clone();
        Ok(ListToolsResult::with_all_items(tools))
    }

    /// Every listed tool answers with its name and arguments.
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if self.get_tool(&request.name).is_none() {
            return Err(ErrorData::invalid_params(
                format!("Unknown tool: {}", request.name),
                None,
            ));
        }
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let text = format!("{} {arguments}", request.name);
        self.calls
            .lock()
            .expect("calls")
            .push((request.name.to_string(), arguments));
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]).into())
    }
}
