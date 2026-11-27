//! The Anthropic format: models, keys, every keyword, signatures, streamed and not.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};

use crate::common::{KEY, anthropic, send};

fn user(text: &str) -> Value {
    json!({
        "model": "fake-chat",
        "max_tokens": 1024,
        "stream": true,
        "messages": [{ "role": "user", "content": [{ "type": "text", "text": text }] }],
    })
}

fn names(events: &[(Option<String>, String)]) -> Vec<String> {
    events
        .iter()
        .map(|(name, _)| name.clone().unwrap_or_default())
        .collect()
}

/// The concatenated `delta.<field>` strings of `content_block_delta` events.
fn joined(chunks: &[Value], field: &str) -> String {
    chunks
        .iter()
        .filter(|chunk| chunk["type"] == "content_block_delta")
        .filter_map(|chunk| chunk["delta"][field].as_str())
        .collect()
}

fn stop(chunks: &[Value]) -> Value {
    chunks
        .iter()
        .find(|chunk| chunk["type"] == "message_delta")
        .map(|chunk| chunk["delta"].clone())
        .expect("a message_delta")
}

#[tokio::test]
async fn lists_the_models_with_their_capabilities() {
    let answer = send(
        Request::get("/v1/models?limit=1000")
            .header("x-api-key", KEY)
            .header("anthropic-version", "2026-10-21")
            .body(Body::empty())
            .expect("a request"),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert!(answer.header("request-id").is_some());
    let body = answer.json();
    assert_eq!(body["has_more"], false);
    assert_eq!(body["data"][0]["type"], "model");
    assert_eq!(body["data"][0]["display_name"], "Fake Chat");
    assert_eq!(body["data"][0]["max_tokens"], 8_192);
    let reasoner = &body["data"][1]["capabilities"];
    assert_eq!(reasoner["thinking"]["types"]["adaptive"]["supported"], true);
    assert_eq!(reasoner["effort"]["high"]["supported"], true);
    assert_eq!(
        body["data"][0]["capabilities"]["thinking"]["types"]["adaptive"]["supported"],
        false
    );
}

#[tokio::test]
async fn needs_the_key_and_the_version() {
    let answer = send(
        Request::post("/v1/messages")
            .header("x-api-key", "wrong")
            .header("anthropic-version", "2026-10-21")
            .body(Body::from(user("hello").to_string()))
            .expect("a request"),
    )
    .await;
    assert_eq!(answer.status, StatusCode::UNAUTHORIZED);
    assert_eq!(answer.json()["error"]["type"], "authentication_error");

    let answer = send(
        Request::post("/v1/messages")
            .header("x-api-key", KEY)
            .body(Body::from(user("hello").to_string()))
            .expect("a request"),
    )
    .await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn streams_named_events_with_you_said() {
    let answer = anthropic(user("hello")).await;
    assert_eq!(answer.status, StatusCode::OK);
    let events = answer.events();
    assert_eq!(
        names(&events)[..3],
        ["message_start", "ping", "content_block_start"]
    );
    assert_eq!(
        names(&events).last().map(String::as_str),
        Some("message_stop")
    );
    let chunks = answer.chunks();
    assert_eq!(joined(&chunks, "text"), "You said: hello");
    assert_eq!(stop(&chunks)["stop_reason"], "end_turn");
    let usage = &chunks[0]["message"]["usage"];
    assert_eq!(usage["input_tokens"], 2);
}

#[tokio::test]
async fn answers_without_streaming() {
    let mut body = user("hello");
    body["stream"] = json!(false);
    let body = anthropic(body).await.json();
    assert_eq!(body["type"], "message");
    assert_eq!(
        body["content"][0],
        json!({ "type": "text", "text": "You said: hello" })
    );
    assert_eq!(body["stop_reason"], "end_turn");
}

#[tokio::test]
async fn failed_answers_the_status_with_an_error_body() {
    let forbidden = anthropic(user("failed 403")).await;
    assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
    assert_eq!(forbidden.json()["error"]["type"], "permission_error");

    let limited = anthropic(user("failed 429")).await;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(limited.header("retry-after"), Some("2"));
    assert_eq!(limited.json()["error"]["type"], "rate_limit_error");

    let overloaded = anthropic(user("failed 529")).await;
    assert_eq!(overloaded.status.as_u16(), 529);
    assert_eq!(overloaded.json()["error"]["type"], "overloaded_error");
}

#[tokio::test]
async fn midstream_error_ends_with_an_error_event() {
    let answer = anthropic(user("midstream error")).await;
    let events = answer.events();
    assert_eq!(names(&events).last().map(String::as_str), Some("error"));
    assert!(!names(&events).contains(&"message_stop".to_owned()));
    assert!(!joined(&answer.chunks(), "text").is_empty());
}

#[tokio::test]
async fn think_streams_a_signed_thinking_block_first() {
    let chunks = anthropic(user("think hello")).await.chunks();
    let first = chunks
        .iter()
        .find(|chunk| chunk["type"] == "content_block_start")
        .expect("a block");
    assert_eq!(first["content_block"]["type"], "thinking");
    let thinking = joined(&chunks, "thinking");
    assert!(thinking.contains("think hello"));
    let signature = joined(&chunks, "signature");
    assert!(signature.starts_with("fake-signature-"));
    assert_eq!(joined(&chunks, "text"), "You said: think hello");
}

#[tokio::test]
async fn a_replayed_thinking_block_needs_its_signature() {
    let thinking = "Some earlier thought.";
    let signed = fake_llm::reply::signature(thinking);
    let history = |signature: &str| {
        json!({
            "model": "fake-reasoner",
            "max_tokens": 1024,
            "messages": [
                { "role": "user", "content": "think" },
                { "role": "assistant", "content": [
                    { "type": "thinking", "thinking": thinking, "signature": signature },
                    { "type": "text", "text": "An answer." },
                ] },
                { "role": "user", "content": "hello" },
            ],
        })
    };
    assert_eq!(anthropic(history(&signed)).await.status, StatusCode::OK);
    let refused = anthropic(history("forged")).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert!(
        refused.json()["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("messages.1.content.0"))
    );
}

#[tokio::test]
async fn the_reasoner_thinks_when_thinking_is_on() {
    let mut body = user("hello");
    body["model"] = json!("fake-reasoner");
    assert!(joined(&anthropic(body.clone()).await.chunks(), "thinking").is_empty());
    body["thinking"] = json!({ "type": "adaptive", "display": "summarized" });
    assert!(!joined(&anthropic(body).await.chunks(), "thinking").is_empty());
}

#[tokio::test]
async fn tool_asks_with_a_tool_use_block() {
    let mut body = user(r#"tool add {"a":1,"b":2}"#);
    body["tools"] = json!([{ "name": "fake__add", "input_schema": { "type": "object" } }]);
    let chunks = anthropic(body).await.chunks();
    let start = chunks
        .iter()
        .find(|chunk| chunk["type"] == "content_block_start")
        .expect("a block");
    assert_eq!(start["content_block"]["type"], "tool_use");
    assert_eq!(start["content_block"]["name"], "fake__add");
    assert!(
        start["content_block"]["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("toolu_fake_"))
    );
    assert_eq!(joined(&chunks, "partial_json"), r#"{"a":1,"b":2}"#);
    assert_eq!(stop(&chunks)["stop_reason"], "tool_use");
}

#[tokio::test]
async fn a_tool_result_gets_tool_said() {
    let body = json!({
        "model": "fake-chat",
        "max_tokens": 1024,
        "stream": true,
        "messages": [
            { "role": "user", "content": "tool echo" },
            { "role": "assistant", "content": [
                { "type": "tool_use", "id": "toolu_1", "name": "echo", "input": {} },
            ] },
            { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "toolu_1", "content": [{ "type": "text", "text": "hi" }] },
            ] },
        ],
    });
    let chunks = anthropic(body).await.chunks();
    assert_eq!(joined(&chunks, "text"), "Tool said: hi");
}

#[tokio::test]
async fn new_text_after_a_skipped_call_is_read_as_the_message() {
    let body = json!({
        "model": "fake-chat",
        "max_tokens": 1024,
        "stream": true,
        "messages": [
            { "role": "user", "content": "tool echo" },
            { "role": "assistant", "content": [
                { "type": "tool_use", "id": "toolu_1", "name": "echo", "input": {} },
            ] },
            { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "toolu_1", "content": "Skipped by user.", "is_error": true },
                { "type": "text", "text": "hello again" },
            ] },
        ],
    });
    let chunks = anthropic(body).await.chunks();
    assert_eq!(joined(&chunks, "text"), "You said: hello again");
}

#[tokio::test]
async fn a_tool_result_without_its_call_is_refused() {
    let body = json!({
        "model": "fake-chat",
        "max_tokens": 1024,
        "messages": [{ "role": "user", "content": [
            { "type": "tool_result", "tool_use_id": "toolu_9", "content": "hi" },
        ] }],
    });
    assert_eq!(anthropic(body).await.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_tool_use_without_its_result_right_after_is_refused() {
    let call = |id: &str| json!({ "type": "tool_use", "id": id, "name": "echo", "input": {} });
    let result = |id: &str| json!({ "type": "tool_result", "tool_use_id": id, "content": "hi" });
    let text = |text: &str| json!({ "type": "text", "text": text });
    let refused = |messages: Value| async move {
        let answer =
            anthropic(json!({ "model": "fake-chat", "max_tokens": 1024, "messages": messages }))
                .await;
        assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{messages}");
        let body = answer.json();
        assert_eq!(body["error"]["type"], "invalid_request_error");
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };

    // The next message is the user's text, not the call's result.
    let message = refused(json!([
        { "role": "user", "content": [text("a")] },
        { "role": "assistant", "content": [call("toolu_1")] },
        { "role": "user", "content": [text("hello")] },
    ]))
    .await;
    assert!(
        message.starts_with("messages.1: `tool_use` ids were found without `tool_result` blocks immediately after: toolu_1."),
        "{message}"
    );

    // One of two calls answered.
    let message = refused(json!([
        { "role": "user", "content": [text("a")] },
        { "role": "assistant", "content": [call("toolu_1"), call("toolu_2")] },
        { "role": "user", "content": [result("toolu_1"), text("b")] },
    ]))
    .await;
    assert!(message.contains("immediately after: toolu_2."), "{message}");

    // The result comes after text: results must come first.
    let message = refused(json!([
        { "role": "user", "content": [text("a")] },
        { "role": "assistant", "content": [call("toolu_1")] },
        { "role": "user", "content": [text("b"), result("toolu_1")] },
    ]))
    .await;
    assert!(message.contains("immediately after: toolu_1."), "{message}");

    // Every call answered first, then text.
    let answer = anthropic(json!({
        "model": "fake-chat",
        "max_tokens": 1024,
        "stream": true,
        "messages": [
            { "role": "user", "content": [text("a")] },
            { "role": "assistant", "content": [call("toolu_1"), call("toolu_2")] },
            { "role": "user", "content": [result("toolu_2"), result("toolu_1"), text("hello")] },
        ],
    }))
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(joined(&answer.chunks(), "text"), "You said: hello");
}

#[tokio::test]
async fn text_written_for_files_is_not_the_message() {
    let chunks = anthropic(json!({
        "model": "fake-chat",
        "max_tokens": 1024,
        "stream": true,
        "messages": [{ "role": "user", "content": [
            { "type": "text", "text": "think hello" },
            { "type": "text", "text": "<file name=\"notes.txt\">\nabc\n</file>" },
        ] }],
    }))
    .await
    .chunks();
    assert!(joined(&chunks, "thinking").contains("think hello"));
    assert_eq!(
        joined(&chunks, "text"),
        "You said: think hello (with 1 attachment)"
    );

    let answer = anthropic(json!({
        "model": "fake-chat",
        "max_tokens": 1024,
        "stream": true,
        "messages": [{ "role": "user", "content": [
            { "type": "text", "text": "failed 429" },
            { "type": "text", "text": "[file \"a.txt\" was deleted]" },
        ] }],
    }))
    .await;
    assert_eq!(answer.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn refuse_stops_with_refusal_details() {
    let chunks = anthropic(user("refuse")).await.chunks();
    let delta = stop(&chunks);
    assert_eq!(delta["stop_reason"], "refusal");
    assert_eq!(delta["stop_details"]["type"], "refusal");
    assert_eq!(
        delta["stop_details"]["explanation"],
        fake_llm::anthropic::REFUSAL
    );
}

#[tokio::test]
async fn refuses_prefill_a_missing_max_tokens_and_an_unknown_model() {
    let prefill = json!({
        "model": "fake-chat",
        "max_tokens": 1024,
        "messages": [
            { "role": "user", "content": "hello" },
            { "role": "assistant", "content": "Sure" },
        ],
    });
    assert_eq!(anthropic(prefill).await.status, StatusCode::BAD_REQUEST);

    let mut missing = user("hello");
    missing
        .as_object_mut()
        .expect("an object")
        .remove("max_tokens");
    assert_eq!(anthropic(missing).await.status, StatusCode::BAD_REQUEST);

    let mut unknown = user("hello");
    unknown["model"] = json!("claude-unknown");
    let answer = anthropic(unknown).await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    assert_eq!(answer.json()["error"]["type"], "not_found_error");
}

#[tokio::test]
async fn reads_only_the_newest_of_merged_user_messages() {
    let body = json!({
        "model": "fake-chat",
        "max_tokens": 1024,
        "stream": true,
        "messages": [{ "role": "user", "content": [
            { "type": "text", "text": "failed 403" },
            { "type": "text", "text": "hello" },
            { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "AAAA" } },
        ] }],
    });
    let chunks = anthropic(body).await.chunks();
    assert_eq!(
        joined(&chunks, "text"),
        "You said: hello (with 1 attachment)"
    );
}
