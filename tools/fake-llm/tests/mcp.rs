//! `/mcp` over HTTP: listing and calling the tools in both protocol eras, and the token guard.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fake_llm::{Config, app};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::common::{Answer, send};

const MODERN: &str = "2026-07-28";

/// A stateless 2026-07-28 request with the per-request metadata.
fn modern(method: &str, params: Value) -> Request<Body> {
    let mut params = params;
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientInfo": { "name": "fake-llm-test", "version": "0.0.0" },
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let mut request = Request::post("/mcp")
        .header("host", "127.0.0.1:4010")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", MODERN)
        .header("mcp-method", method);
    if let Some(name) = name {
        request = request.header("mcp-name", name);
    }
    request
        .body(Body::from(body.to_string()))
        .expect("a request")
}

/// The JSON-RPC result of a JSON or SSE answer.
fn result(answer: &Answer) -> Value {
    let message: Value = if answer.body.trim_start().starts_with('{') {
        answer.json()
    } else {
        answer
            .chunks()
            .into_iter()
            .find(|chunk| chunk.get("result").is_some() || chunk.get("error").is_some())
            .expect("a JSON-RPC answer")
    };
    assert!(message.get("error").is_none(), "an error: {message}");
    message["result"].clone()
}

#[tokio::test]
async fn lists_echo_and_add_statelessly() {
    let answer = send(modern("tools/list", json!({}))).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    let tools = result(&answer)["tools"].clone();
    let names: Vec<&str> = tools
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_eq!(names, ["echo", "add"]);
}

#[tokio::test]
async fn calls_echo_and_add() {
    let echo = send(modern(
        "tools/call",
        json!({ "name": "echo", "arguments": { "text": "smoke" } }),
    ))
    .await;
    assert_eq!(result(&echo)["content"][0]["text"], "smoke");
    let add = send(modern(
        "tools/call",
        json!({ "name": "add", "arguments": { "a": 2, "b": 3 } }),
    ))
    .await;
    assert_eq!(result(&add)["content"][0]["text"], "5");
    let wrong = send(modern(
        "tools/call",
        json!({ "name": "add", "arguments": { "a": "x" } }),
    ))
    .await;
    assert_eq!(result(&wrong)["isError"], true);
}

#[tokio::test]
async fn older_clients_get_a_session() {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "fake-llm-test", "version": "0.0.0" },
        },
    });
    let request = Request::post("/mcp")
        .header("host", "127.0.0.1:4010")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(Body::from(body.to_string()))
        .expect("a request");
    let response = common::server().oneshot(request).await.expect("infallible");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().contains_key("mcp-session-id"));
    // The first SSE event carries the answer; the stream itself stays open for the session.
    let mut body = response.into_body();
    let mut text = String::new();
    while !text.contains("\"result\"") {
        let frame = body.frame().await.expect("a frame").expect("a body");
        if let Ok(data) = frame.into_data() {
            text.push_str(&String::from_utf8_lossy(&data));
        }
    }
    assert!(text.contains("2025-06-18"), "{text}");
}

#[tokio::test]
async fn a_token_guards_the_tools() {
    let guarded = || {
        app(Config {
            mcp_token: Some("secret".to_owned()),
            ..Config::default()
        })
    };
    let refused = guarded()
        .oneshot(modern("tools/list", json!({})))
        .await
        .expect("infallible");
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    let mut request = modern("tools/list", json!({}));
    request
        .headers_mut()
        .insert("authorization", "Bearer secret".parse().expect("a header"));
    let allowed = guarded().oneshot(request).await.expect("infallible");
    assert_eq!(allowed.status(), StatusCode::OK);
}
