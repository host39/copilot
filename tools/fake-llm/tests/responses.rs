//! The Responses format: keys, models, every keyword, the stateless checks, streamed and not.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fake_llm::responses::{REFUSAL, encrypt};
use serde_json::{Value, json};

use crate::common::{Answer, responses, send};

fn user(text: &str) -> Value {
    json!({
        "model": "fake-chat",
        "stream": true,
        "store": false,
        "instructions": "Be brief.",
        "input": [{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": text }] }],
    })
}

/// The events of a stream of the given type.
fn of<'a>(events: &'a [Value], kind: &str) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|event| event["type"] == kind)
        .collect()
}

/// The concatenated `delta` strings of the events of `kind`.
fn joined(events: &[Value], kind: &str) -> String {
    of(events, kind)
        .iter()
        .filter_map(|event| event["delta"].as_str())
        .collect()
}

/// The data of every event, after checking that each event's name is its data's `type` and
/// that the sequence numbers count up from 0.
fn streamed(answer: &Answer) -> Vec<Value> {
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    let mut out = Vec::new();
    for (index, (name, data)) in answer.events().into_iter().enumerate() {
        let data: Value = serde_json::from_str(&data).expect("JSON data");
        assert_eq!(name.as_deref(), data["type"].as_str());
        assert_eq!(data["sequence_number"], index);
        out.push(data);
    }
    out
}

fn last(events: &[Value]) -> &Value {
    events.last().expect("an event")
}

#[tokio::test]
async fn refuses_a_missing_key_and_unknown_models() {
    let answer = send(
        Request::post("/v1/responses")
            .header("content-type", "application/json")
            .body(Body::from(user("hello").to_string()))
            .expect("a request"),
    )
    .await;
    assert_eq!(answer.status, StatusCode::UNAUTHORIZED);
    assert_eq!(answer.json()["error"]["code"], "invalid_api_key");

    let mut body = user("hello");
    body["model"] = json!("gpt-unknown");
    let answer = responses(body).await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    assert_eq!(answer.json()["error"]["code"], "model_not_found");
}

#[tokio::test]
async fn streams_text_with_usage_in_typed_events() {
    let answer = responses(user("hello")).await;
    assert!(answer.header("x-request-id").is_some());
    let events = streamed(&answer);
    let kinds: Vec<&str> = events
        .iter()
        .filter_map(|event| event["type"].as_str())
        .collect();
    assert_eq!(
        kinds[..3],
        [
            "response.created",
            "response.in_progress",
            "response.output_item.added"
        ]
    );
    assert_eq!(
        joined(&events, "response.output_text.delta"),
        "You said: hello"
    );
    let done = last(&events);
    assert_eq!(done["type"], "response.completed");
    assert_eq!(done["response"]["status"], "completed");
    assert_eq!(done["response"]["store"], false);
    let output = &done["response"]["output"][0];
    assert_eq!(output["type"], "message");
    assert_eq!(output["content"][0]["text"], "You said: hello");
    assert!(done["response"]["usage"]["output_tokens"].as_u64() > Some(0));
}

#[tokio::test]
async fn thinks_with_a_summary_and_encrypted_reasoning() {
    let answer = responses(user("think about it")).await;
    let events = streamed(&answer);
    let summary = joined(&events, "response.reasoning_summary_text.delta");
    assert!(summary.starts_with("The user wrote"), "{summary}");
    let reasoning = &of(&events, "response.output_item.done")[0]["item"];
    assert_eq!(reasoning["type"], "reasoning");
    assert_eq!(
        reasoning["encrypted_content"],
        encrypt("fake-chat", &summary)
    );
    assert!(
        of(&events, "response.output_item.added")[0]["item"]
            .get("encrypted_content")
            .is_none(),
        "only the finished item carries it"
    );

    // fake-reasoner reasons unless the effort is none.
    let mut reasoner = user("hello");
    reasoner["model"] = json!("fake-reasoner");
    let answer = responses(reasoner.clone()).await;
    assert!(!joined(&streamed(&answer), "response.reasoning_summary_text.delta").is_empty());
    reasoner["reasoning"] = json!({ "effort": "none" });
    let answer = responses(reasoner).await;
    assert!(joined(&streamed(&answer), "response.reasoning_summary_text.delta").is_empty());
}

#[tokio::test]
async fn calls_a_tool_and_reads_its_output_in_the_next_request() {
    let mut body = user(r#"think tool echo {"text":"hi"}"#);
    body["tools"] = json!([{ "type": "function", "name": "fake__echo", "parameters": { "type": "object" }, "strict": false }]);
    let answer = responses(body.clone()).await;
    let events = streamed(&answer);
    let added = &of(&events, "response.output_item.added")[1]["item"];
    assert_eq!(added["type"], "function_call");
    assert_eq!(added["name"], "fake__echo");
    assert_eq!(added["arguments"], "");
    assert_eq!(
        joined(&events, "response.function_call_arguments.delta"),
        r#"{"text":"hi"}"#
    );
    let output = last(&events)["response"]["output"].clone();
    let call_id = output[1]["call_id"].as_str().expect("a call id").to_owned();

    // The next request replays the items and answers the call.
    let mut input = body["input"].as_array().expect("input").clone();
    input.extend(output.as_array().expect("items").iter().cloned());
    input.push(json!({ "type": "function_call_output", "call_id": call_id, "output": "hi" }));
    body["input"] = Value::Array(input.clone());
    let answer = responses(body.clone()).await;
    assert_eq!(
        joined(&streamed(&answer), "response.output_text.delta"),
        "Tool said: hi"
    );

    // A call without its output, an output without its call, and changed or missing encrypted
    // reasoning are refused, as the real API refuses them.
    let mut unanswered = body.clone();
    unanswered["input"] = Value::Array(input[..input.len() - 1].to_vec());
    let answer = responses(unanswered).await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        answer.json()["error"]["message"],
        format!("No tool output found for function call {call_id}.")
    );
    let mut orphan = body.clone();
    orphan["input"] = json!([input[0].clone(), input[input.len() - 1].clone()]);
    let answer = responses(orphan).await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    let mut changed = body.clone();
    changed["input"][1]["encrypted_content"] = json!("tampered");
    assert_eq!(responses(changed).await.status, StatusCode::BAD_REQUEST);
    let mut missing = body.clone();
    missing["input"][1]
        .as_object_mut()
        .expect("an item")
        .remove("encrypted_content");
    let answer = responses(missing).await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    assert!(
        answer.json()["error"]["message"]
            .as_str()
            .expect("a message")
            .contains("Items are not persisted when `store` is set to false")
    );

    // The reasoning went to the model that wrote it; another model refuses it, and a model
    // without reasoning says it takes none.
    let mut audio = body.clone();
    audio["model"] = json!("fake-audio");
    let answer = responses(audio).await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    assert!(
        answer.json()["error"]["message"]
            .as_str()
            .expect("a message")
            .starts_with("Reasoning input items can only be provided to a reasoning")
    );
    let mut reasoner = body;
    reasoner["model"] = json!("fake-reasoner");
    let answer = responses(reasoner).await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    assert!(
        answer.json()["error"]["message"]
            .as_str()
            .expect("a message")
            .contains("could not be verified")
    );
}

#[tokio::test]
async fn refuses_after_a_little_text() {
    let events = streamed(&responses(user("refuse")).await);
    assert_eq!(
        joined(&events, "response.output_text.delta"),
        "I would rather not"
    );
    assert_eq!(joined(&events, "response.refusal.delta"), REFUSAL);
    let message = &last(&events)["response"]["output"][0];
    assert_eq!(
        message["content"][1],
        json!({ "type": "refusal", "refusal": REFUSAL })
    );
}

#[tokio::test]
async fn fails_with_a_status_or_mid_stream() {
    let answer = responses(user("failed 429")).await;
    assert_eq!(answer.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(answer.header("retry-after"), Some("2"));
    assert_eq!(answer.json()["error"]["code"], "rate_limit_exceeded");

    let events = streamed(&responses(user("midstream error")).await);
    let failed = last(&events);
    assert_eq!(failed["type"], "response.failed");
    assert_eq!(failed["response"]["error"]["code"], "server_error");
    assert!(of(&events, "response.output_item.done").is_empty());

    let mut whole = user("midstream error");
    whole["stream"] = json!(false);
    assert_eq!(
        responses(whole).await.status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[tokio::test]
async fn answers_whole_requests_and_string_input() {
    let body = json!({ "model": "fake-chat", "input": "hello there" });
    let answer = responses(body).await;
    assert_eq!(answer.status, StatusCode::OK);
    let response = answer.json();
    assert_eq!(response["object"], "response");
    assert_eq!(
        response["output"][0]["content"][0]["text"],
        "You said: hello there"
    );

    let stateful = json!({ "model": "fake-chat", "input": "hi", "previous_response_id": "resp_1" });
    assert_eq!(responses(stateful).await.status, StatusCode::NOT_FOUND);
    let empty = json!({ "model": "fake-chat", "input": [] });
    assert_eq!(responses(empty).await.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn counts_attachments_and_ignores_copilots_file_notices() {
    let mut body = user("look");
    body["input"][0]["content"] = json!([
        { "type": "input_image", "image_url": "data:image/png;base64,iVBORw==" },
        { "type": "input_text", "text": "look" },
        { "type": "input_file", "filename": "a.pdf", "file_data": "data:application/pdf;base64,JVBERg==" },
    ]);
    let events = streamed(&responses(body).await);
    assert_eq!(
        joined(&events, "response.output_text.delta"),
        "You said: look (with 1 attachment)"
    );
}
