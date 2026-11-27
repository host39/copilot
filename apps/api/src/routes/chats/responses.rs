//! A chat on a connection that uses the Responses API, end to end (`make test-db`): wiremock
//! answers `POST /v1/responses` with OpenAI's typed events, the in-process rmcp server runs the
//! tool, and core's replay rules decide what goes back. The tool round sends the answer's
//! encrypted reasoning and function call back with the result, and the next turn replays the
//! earlier answer again while its context is unchanged.

use axum::Router;
use axum::http::Method;
use copilot_database::DbPool;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::helpers::mcp_server::McpServer;
use crate::helpers::testing::{self, config, live, register};
use crate::routes::chats::flows::{call, content, messages, say, text};

/// An SSE body of named events, each event's name repeated in its data's `type`.
fn sse(events: &[Value]) -> ResponseTemplate {
    let body: String = events
        .iter()
        .map(|event| {
            let name = event["type"].as_str().expect("a type");
            format!("event: {name}\ndata: {event}\n\n")
        })
        .collect();
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

fn reasoning() -> Value {
    json!({ "id": "rs_1", "type": "reasoning", "status": "completed",
        "summary": [{ "type": "summary_text", "text": "Searching first." }],
        "encrypted_content": "gAAAAB-opaque" })
}

fn function_call() -> Value {
    json!({ "id": "fc_1", "type": "function_call", "status": "completed", "call_id": "call_abc",
        "name": "docs__search", "arguments": "{\"q\":\"rust\"}" })
}

fn message(text: &str) -> Value {
    json!({ "id": "msg_1", "type": "message", "role": "assistant", "status": "completed",
        "content": [{ "type": "output_text", "text": text, "annotations": [] }] })
}

fn completed(output: &[Value]) -> Value {
    json!({ "type": "response.completed", "response": {
        "id": "resp_1", "status": "completed", "output": output,
        "usage": { "input_tokens": 40, "output_tokens": 10 },
    } })
}

/// Reasoning, then a call of `docs__search`, each item streamed and finished.
fn calling() -> Vec<Value> {
    vec![
        json!({ "type": "response.output_item.added", "output_index": 0, "item": { "id": "rs_1", "type": "reasoning", "summary": [] } }),
        json!({ "type": "response.reasoning_summary_text.delta", "output_index": 0, "summary_index": 0, "delta": "Searching first." }),
        json!({ "type": "response.output_item.done", "output_index": 0, "item": reasoning() }),
        json!({ "type": "response.output_item.added", "output_index": 1, "item": { "id": "fc_1", "type": "function_call", "call_id": "call_abc", "name": "docs__search", "arguments": "" } }),
        json!({ "type": "response.function_call_arguments.delta", "output_index": 1, "delta": "{\"q\":\"rust\"}" }),
        json!({ "type": "response.output_item.done", "output_index": 1, "item": function_call() }),
        completed(&[reasoning(), function_call()]),
    ]
}

/// A streamed text answer.
fn answer(text: &str) -> Vec<Value> {
    vec![
        json!({ "type": "response.output_item.added", "output_index": 0, "item": { "id": "msg_1", "type": "message", "role": "assistant", "content": [] } }),
        json!({ "type": "response.output_text.delta", "output_index": 0, "content_index": 0, "delta": text }),
        json!({ "type": "response.output_item.done", "output_index": 0, "item": message(text) }),
        completed(&[message(text)]),
    ]
}

async fn script(provider: &MockServer, events: Vec<Value>) {
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(sse(&events))
        .up_to_n_times(1)
        .mount(provider)
        .await;
}

/// The Responses request bodies the provider received, in order.
async fn sent(provider: &MockServer) -> Vec<Value> {
    provider
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| request.url.path() == "/v1/responses")
        .map(|request| serde_json::from_slice(&request.body).expect("a JSON body"))
        .collect()
}

/// A `custom` connection to `provider` on the Responses API, refreshed: its model's id.
async fn responses_model(app: &Router, token: &str, provider: &MockServer) -> String {
    let created = call(
        app,
        token,
        Method::POST,
        "/api/providers",
        Some(json!({
            "kind": "openai",
            "preset": "custom",
            "api": "responses",
            "name": "Gateway",
            "baseUrl": format!("{}/v1", provider.uri()),
        })),
    )
    .await;
    assert_eq!(created["data"]["api"], "responses");
    let id = created["data"]["id"].as_str().expect("an id");
    call(
        app,
        token,
        Method::POST,
        &format!("/api/providers/{id}/refresh"),
        None,
    )
    .await;
    let models = call(
        app,
        token,
        Method::GET,
        &format!("/api/providers/{id}/models"),
        None,
    )
    .await;
    models["items"][0]["id"]
        .as_str()
        .expect("a model")
        .to_owned()
}

/// The tool server "Docs" with `search`, set to run without asking; the server and its id.
async fn docs(app: &Router, token: &str) -> (McpServer, String) {
    let mcp = McpServer::start(
        false,
        &[(
            "search".to_owned(),
            "Search the docs.".to_owned(),
            json!({ "type": "object", "properties": { "q": { "type": "string" } } }),
        )],
        None,
    )
    .await;
    let created = call(
        app,
        token,
        Method::POST,
        "/api/tool-servers",
        Some(json!({ "name": "Docs", "url": mcp.url, "approval": "auto" })),
    )
    .await;
    let id = created["data"]["id"].as_str().expect("an id").to_owned();
    (mcp, id)
}

/// The input items of `body` of `kind`.
fn items<'a>(body: &'a Value, kind: &str) -> Vec<&'a Value> {
    body["input"]
        .as_array()
        .expect("input")
        .iter()
        .filter(|item| item["type"] == kind)
        .collect()
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn a_tool_round_on_the_responses_api_replays_encrypted_reasoning(pool: DbPool) {
    let provider = testing::provider().await;
    let app = live(pool, config()).await;
    let app = app.router;
    let token = register(&app, "admin").await;
    let model = responses_model(&app, &token, &provider).await;
    let (mcp, server) = docs(&app, &token).await;
    let chat = call(
        &app,
        &token,
        Method::POST,
        "/api/chats",
        Some(json!({
            "modelId": model,
            "toolOverrides": [{ "serverId": server, "enabled": true }],
        })),
    )
    .await;
    let chat = chat["data"]["id"].as_str().expect("an id").to_owned();
    script(&provider, calling()).await;
    script(&provider, answer("Found it.")).await;

    let events = say(&app, &token, &chat, "Look it up").await;
    assert!(
        events
            .iter()
            .any(|event| event["type"] == "reasoning.delta" && event["text"] == "Searching first."),
        "the reasoning summary streams"
    );
    assert_eq!(text(&events), "Found it.");
    assert_eq!(mcp.calls(), [("search".to_owned(), json!({ "q": "rust" }))]);
    let saved = messages(&app, &token, &chat).await;
    assert_eq!(saved[1]["finishReason"], "tool_calls");
    assert_eq!(content(&saved[3]), "Found it.");

    let bodies = sent(&provider).await;
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0]["store"], false);
    assert_eq!(bodies[0]["tools"][0]["name"], "docs__search");
    let round = &bodies[1];
    assert_eq!(
        items(round, "reasoning"),
        [&reasoning()],
        "encrypted reasoning goes back"
    );
    assert_eq!(items(round, "function_call"), [&function_call()]);
    let outputs = items(round, "function_call_output");
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0]["call_id"], "call_abc", "the provider's own id");

    // The next turn keeps the context, so the earlier answers replay again.
    script(&provider, answer("You're welcome.")).await;
    let events = say(&app, &token, &chat, "Thanks").await;
    assert_eq!(text(&events), "You're welcome.");
    let bodies = sent(&provider).await;
    let next = &bodies[2];
    assert_eq!(items(next, "reasoning"), [&reasoning()]);
    assert_eq!(
        items(next, "message")
            .iter()
            .filter(|item| item["role"] == "assistant")
            .count(),
        1,
        "the stored answer message"
    );
    assert_eq!(
        items(next, "message").last().expect("the new message")["content"][0]["text"],
        "Thanks"
    );
}
