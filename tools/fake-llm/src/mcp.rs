//! A tiny MCP server at `/mcp`: rmcp's own Streamable HTTP server with two tools, so the tool
//! flow (listing, approval, call, result) can be tried by hand and by the smoke test.
//!
//! - `echo {text}` answers with `text` unchanged.
//! - `add {a, b}` answers with the sum; arguments that are not two numbers give an error result.
//!
//! It speaks both protocol eras (stateless 2026-07-28 requests, and sessions for older
//! clients), answers modern requests with JSON, and accepts any `Host` (a dev tool, normally on
//! loopback). With `FAKE_LLM_MCP_TOKEN` set it answers `401` without that bearer token.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, ServerHandler};
use serde_json::{Map, Value, json};

/// The MCP routes; `token` guards them when set.
pub fn router(token: Option<String>) -> Router {
    let config = StreamableHttpServerConfig::default()
        .with_json_response(true)
        .disable_allowed_hosts();
    let service: StreamableHttpService<FakeTools, LocalSessionManager> =
        StreamableHttpService::new(|| Ok(FakeTools), Arc::default(), config);
    let router = Router::new().nest_service("/mcp", service);
    match token {
        Some(token) => router.layer(from_fn_with_state(Arc::new(token), guard)),
        None => router,
    }
}

/// Answers `401` unless the request carries `Authorization: Bearer <token>`.
async fn guard(State(token): State<Arc<String>>, request: Request, next: Next) -> Response {
    let expected = format!("Bearer {token}");
    let given = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if given == Some(expected.as_str()) {
        return next.run(request).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        "fake-llm: the MCP server needs the bearer token set in FAKE_LLM_MCP_TOKEN",
    )
        .into_response()
}

/// The tool server: stateless, so every session shares it.
#[derive(Clone, Copy, Debug)]
pub struct FakeTools;

/// The listed tools.
pub fn tools() -> Vec<Tool> {
    let mut echo = Tool::new(
        "echo",
        "Answers with the given text, unchanged.",
        schema(
            json!({ "text": { "type": "string", "description": "The text to send back." } }),
            &["text"],
        ),
    );
    echo.annotations = Some(ToolAnnotations::with_title("Echo").read_only(true));
    let mut add = Tool::new(
        "add",
        "Adds two numbers.",
        schema(
            json!({ "a": { "type": "number" }, "b": { "type": "number" } }),
            &["a", "b"],
        ),
    );
    add.annotations = Some(ToolAnnotations::with_title("Add").read_only(true));
    vec![echo, add]
}

fn schema(properties: Value, required: &[&str]) -> Map<String, Value> {
    let Value::Object(object) = json!({
        "type": "object",
        "properties": properties,
        "required": required,
    }) else {
        unreachable!("an object literal");
    };
    object
}

/// Runs one tool on its arguments; `None` for a tool that is not listed.
pub fn run(name: &str, arguments: Option<&Map<String, Value>>) -> Option<CallToolResult> {
    let argument = |key: &str| arguments.and_then(|arguments| arguments.get(key));
    Some(match name {
        "echo" => match argument("text").and_then(Value::as_str) {
            Some(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
            None => CallToolResult::error(vec![ContentBlock::text("echo needs a string `text`.")]),
        },
        "add" => match (
            argument("a").and_then(Value::as_f64),
            argument("b").and_then(Value::as_f64),
        ) {
            (Some(a), Some(b)) => CallToolResult::success(vec![ContentBlock::text(sum(a + b))]),
            _ => CallToolResult::error(vec![ContentBlock::text(
                "add needs two numbers, `a` and `b`.",
            )]),
        },
        _ => return None,
    })
}

/// A sum without a trailing `.0` when it is whole.
fn sum(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

impl ServerHandler for FakeTools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        tools().into_iter().find(|tool| tool.name == name)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        tracing::info!(tool = %request.name, "fake-llm.mcp.called");
        match run(&request.name, request.arguments.as_ref()) {
            Some(result) => Ok(result.into()),
            None => Err(ErrorData::invalid_params(
                format!("Unknown tool: {}", request.name),
                None,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(result: &CallToolResult) -> String {
        let value = serde_json::to_value(result).expect("serializable");
        value["content"][0]["text"]
            .as_str()
            .expect("text")
            .to_owned()
    }

    fn arguments(value: Value) -> Map<String, Value> {
        let Value::Object(map) = value else {
            panic!("an object");
        };
        map
    }

    #[test]
    fn echo_sends_its_text_back() {
        let result =
            run("echo", Some(&arguments(json!({ "text": "smoke check" })))).expect("listed");
        assert_eq!(text(&result), "smoke check");
        assert_ne!(result.is_error, Some(true));
        let missing = run("echo", None).expect("listed");
        assert_eq!(missing.is_error, Some(true));
    }

    #[test]
    fn add_sums_two_numbers() {
        let whole = run("add", Some(&arguments(json!({ "a": 2, "b": 40 })))).expect("listed");
        assert_eq!(text(&whole), "42");
        let fraction = run("add", Some(&arguments(json!({ "a": 0.5, "b": 1 })))).expect("listed");
        assert_eq!(text(&fraction), "1.5");
        let wrong = run("add", Some(&arguments(json!({ "a": "two", "b": 1 })))).expect("listed");
        assert_eq!(wrong.is_error, Some(true));
    }

    #[test]
    fn lists_echo_and_add_only() {
        let names: Vec<String> = tools().iter().map(|tool| tool.name.to_string()).collect();
        assert_eq!(names, ["echo", "add"]);
        assert!(run("other", None).is_none());
    }
}
