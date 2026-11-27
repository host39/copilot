//! The OpenAI-compatible format: models, keys, every keyword, streamed and not.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use crate::common::{KEY, openai, send};

fn user(text: &str) -> Value {
    json!({ "model": "fake-chat", "stream": true, "messages": [{ "role": "user", "content": text }] })
}

/// The concatenated `delta.<field>` strings of a stream.
fn joined(chunks: &[Value], field: &str) -> String {
    chunks
        .iter()
        .filter_map(|chunk| {
            chunk
                .pointer(&format!("/choices/0/delta/{field}"))?
                .as_str()
        })
        .collect()
}

fn finish(chunks: &[Value]) -> Vec<String> {
    chunks
        .iter()
        .filter_map(|chunk| chunk.pointer("/choices/0/finish_reason")?.as_str())
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn lists_the_models_with_their_capabilities() {
    let answer = send(
        Request::get("/v1/models")
            .header("authorization", format!("Bearer {KEY}"))
            .body(Body::empty())
            .expect("a request"),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert!(
        answer
            .header("x-request-id")
            .is_some_and(|id| id.starts_with("req_fake_"))
    );
    let body = answer.json();
    let ids: Vec<&str> = body["data"]
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|model| model["id"].as_str())
        .collect();
    assert_eq!(ids, ["fake-chat", "fake-reasoner", "fake-audio"]);
    let audio = &body["data"][2];
    assert_eq!(
        audio["architecture"]["input_modalities"],
        json!(["text", "audio"])
    );
    assert_eq!(
        audio["architecture"]["output_modalities"],
        json!(["text", "audio"])
    );
    let reasoner = &body["data"][1];
    assert!(
        reasoner["supported_parameters"]
            .as_array()
            .expect("a list")
            .contains(&json!("reasoning"))
    );
    assert_eq!(body["data"][0]["context_length"], 128_000);
}

#[tokio::test]
async fn refuses_a_missing_or_wrong_key() {
    for header in [None, Some("Bearer wrong")] {
        let mut request = Request::get("/v1/models");
        if let Some(value) = header {
            request = request.header("authorization", value);
        }
        let answer = send(request.body(Body::empty()).expect("a request")).await;
        assert_eq!(answer.status, StatusCode::UNAUTHORIZED);
        assert_eq!(answer.json()["error"]["code"], "invalid_api_key");
    }
}

#[tokio::test]
async fn streams_you_said_in_chunks_with_usage_and_done() {
    let answer = openai(user("hello")).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert!(
        answer
            .header("content-type")
            .is_some_and(|value| value.starts_with("text/event-stream"))
    );
    let events = answer.events();
    assert_eq!(events.last().map(|(_, data)| data.as_str()), Some("[DONE]"));
    let chunks = answer.chunks();
    assert_eq!(joined(&chunks, "content"), "You said: hello");
    let texts = chunks
        .iter()
        .filter(|chunk| {
            chunk
                .pointer("/choices/0/delta/content")
                .is_some_and(|text| text != "")
        })
        .count();
    assert!(texts >= 3, "a few chunks, got {texts}");
    assert_eq!(finish(&chunks), ["stop"]);
    let usage = chunks
        .iter()
        .find_map(|chunk| chunk.get("usage"))
        .expect("a usage chunk");
    assert_eq!(usage["prompt_tokens"], 2);
    assert_eq!(usage["completion_tokens"], 4);
    assert_eq!(chunks[0]["object"], "chat.completion.chunk");
}

#[tokio::test]
async fn answers_without_streaming() {
    let mut body = user("hello");
    body["stream"] = json!(false);
    let answer = openai(body).await;
    assert_eq!(answer.status, StatusCode::OK);
    let body = answer.json();
    assert_eq!(body["object"], "chat.completion");
    assert_eq!(body["choices"][0]["message"]["content"], "You said: hello");
    assert_eq!(body["choices"][0]["finish_reason"], "stop");
    assert_eq!(body["usage"]["total_tokens"], 6);
}

#[tokio::test]
async fn failed_answers_the_status_with_an_error_body() {
    let forbidden = openai(user("failed 403")).await;
    assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
    assert!(
        forbidden.json()["error"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("403"))
    );

    let limited = openai(user("failed 429")).await;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(limited.header("retry-after"), Some("2"));
    assert_eq!(limited.json()["error"]["code"], "rate_limit_exceeded");

    let broken = openai(user("failed 500")).await;
    assert_eq!(broken.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(broken.json()["error"]["type"], "server_error");
}

#[tokio::test]
async fn midstream_error_breaks_off_without_done() {
    let answer = openai(user("midstream error")).await;
    assert_eq!(answer.status, StatusCode::OK);
    let chunks = answer.chunks();
    assert!(!joined(&chunks, "content").is_empty());
    let last = chunks.last().expect("chunks");
    assert_eq!(last["error"]["type"], "server_error");
    assert!(answer.events().iter().all(|(_, data)| data != "[DONE]"));
    assert!(finish(&chunks).is_empty());

    let mut whole = user("midstream error");
    whole["stream"] = json!(false);
    assert_eq!(
        openai(whole).await.status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[tokio::test]
async fn think_streams_reasoning_content_before_the_text() {
    let chunks = openai(user("think hello")).await.chunks();
    let reasoning = chunks
        .iter()
        .position(|chunk| {
            chunk
                .pointer("/choices/0/delta/reasoning_content")
                .is_some()
        })
        .expect("reasoning");
    let text = chunks
        .iter()
        .position(|chunk| {
            chunk
                .pointer("/choices/0/delta/content")
                .is_some_and(|text| text != "")
        })
        .expect("text");
    assert!(reasoning < text);
    assert!(joined(&chunks, "reasoning_content").contains("think hello"));
}

#[tokio::test]
async fn the_reasoner_reasons_unless_reasoning_is_off() {
    let mut body = user("hello");
    body["model"] = json!("fake-reasoner");
    let chunks = openai(body.clone()).await.chunks();
    assert!(!joined(&chunks, "reasoning_content").is_empty());
    body["reasoning_effort"] = json!("none");
    let chunks = openai(body).await.chunks();
    assert!(joined(&chunks, "reasoning_content").is_empty());
}

#[tokio::test]
async fn tool_asks_for_the_offered_tool_by_alias() {
    let mut body = user(r#"tool echo {"text":"hi"}"#);
    body["tools"] = json!([
        { "type": "function", "function": { "name": "fake__add", "parameters": {} } },
        { "type": "function", "function": { "name": "fake__echo", "parameters": {} } },
    ]);
    let chunks = openai(body).await.chunks();
    let calls: Vec<&Value> = chunks
        .iter()
        .filter_map(|chunk| chunk.pointer("/choices/0/delta/tool_calls/0"))
        .collect();
    assert_eq!(calls[0]["function"]["name"], "fake__echo");
    assert!(
        calls[0]["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("call_fake_"))
    );
    let arguments: String = calls
        .iter()
        .filter_map(|call| call.pointer("/function/arguments")?.as_str())
        .collect();
    assert_eq!(arguments, r#"{"text":"hi"}"#);
    assert!(calls.len() > 2, "arguments arrive in fragments");
    assert_eq!(finish(&chunks), ["tool_calls"]);
}

#[tokio::test]
async fn a_tool_result_gets_tool_said() {
    let body = json!({
        "model": "fake-chat",
        "stream": true,
        "messages": [
            { "role": "user", "content": "tool echo {\"text\":\"hi\"}" },
            { "role": "assistant", "content": null, "tool_calls": [
                { "id": "call_1", "type": "function", "function": { "name": "echo", "arguments": "{}" } },
            ] },
            { "role": "tool", "tool_call_id": "call_1", "content": "hi" },
        ],
    });
    let chunks = openai(body).await.chunks();
    assert_eq!(joined(&chunks, "content"), "Tool said: hi");
}

#[tokio::test]
async fn a_tool_message_without_its_call_is_refused() {
    let body = json!({
        "model": "fake-chat",
        "messages": [
            { "role": "user", "content": "hello" },
            { "role": "tool", "tool_call_id": "call_1", "content": "hi" },
        ],
    });
    let answer = openai(body).await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    assert_eq!(answer.json()["error"]["type"], "invalid_request_error");
}

#[tokio::test]
async fn refuse_ends_with_content_filter() {
    let chunks = openai(user("refuse")).await.chunks();
    assert_eq!(finish(&chunks), ["content_filter"]);
}

#[tokio::test]
async fn long_streams_a_long_reply() {
    let chunks = openai(user("long")).await.chunks();
    let text = joined(&chunks, "content");
    assert!(text.starts_with("# A Long Reply") && text.len() > 5_000);
}

#[tokio::test]
async fn audio_streams_a_tone_with_its_transcript() {
    let body = json!({
        "model": "fake-audio",
        "stream": true,
        "modalities": ["text", "audio"],
        "audio": { "voice": "alloy", "format": "pcm16" },
        "messages": [{ "role": "user", "content": "audio" }],
    });
    let chunks = openai(body).await.chunks();
    let audio: Vec<&Value> = chunks
        .iter()
        .filter_map(|chunk| chunk.pointer("/choices/0/delta/audio"))
        .collect();
    let pcm: Vec<u8> = audio
        .iter()
        .filter_map(|piece| piece["data"].as_str())
        .flat_map(|data| STANDARD.decode(data).expect("base64"))
        .collect();
    assert_eq!(pcm.len(), 48_000, "one second of 24 kHz PCM16");
    let transcript: String = audio
        .iter()
        .filter_map(|piece| piece["transcript"].as_str())
        .collect();
    assert_eq!(transcript, "Here is a short tone.");
    assert!(
        audio
            .last()
            .is_some_and(|piece| piece.get("expires_at").is_some())
    );
    assert_eq!(finish(&chunks), ["stop"]);

    let mut plain = user("audio");
    plain["model"] = json!("fake-audio");
    let text = joined(&openai(plain).await.chunks(), "content");
    assert!(text.contains("Speak replies"), "no audio asked: {text}");
}

#[tokio::test]
async fn audio_without_streaming_follows_the_requested_format() {
    let body = json!({
        "model": "fake-audio",
        "modalities": ["text", "audio"],
        "audio": { "voice": "alloy", "format": "wav" },
        "messages": [{ "role": "user", "content": "audio" }],
    });
    let body = openai(body).await.json();
    let data = body["choices"][0]["message"]["audio"]["data"]
        .as_str()
        .expect("audio");
    let wav = STANDARD.decode(data).expect("base64");
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(wav.len(), 48_044);
}

#[tokio::test]
async fn names_attachments_of_the_last_user_message() {
    let body = json!({
        "model": "fake-chat",
        "stream": true,
        "messages": [{ "role": "user", "content": [
            { "type": "text", "text": "what is this" },
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } },
        ] }],
    });
    let chunks = openai(body).await.chunks();
    assert_eq!(
        joined(&chunks, "content"),
        "You said: what is this (with 1 attachment)"
    );
}

#[tokio::test]
async fn refuses_an_unknown_model_and_a_bad_body() {
    let mut body = user("hello");
    body["model"] = json!("gpt-unknown");
    let answer = openai(body).await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    assert_eq!(answer.json()["error"]["code"], "model_not_found");

    let answer = openai(json!({ "model": "fake-chat", "messages": [] })).await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
}
