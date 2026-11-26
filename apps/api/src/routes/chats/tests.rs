//! The walking skeleton end to end (`make test-db`): rustauth and Copilot on a throwaway Postgres
//! database, and a wiremock server as the OpenAI-compatible provider, reached through the real
//! adapter and the hardened clients. The first account is an administrator, so its connection
//! may use the loopback address of the mock (`ALLOW_PRIVATE_NETWORK=admins`).

use axum::Router;
use axum::http::header::CONTENT_TYPE;
use axum::http::{Method, StatusCode};
use copilot_database::DbPool;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::helpers::testing::{live_app, read, register, request, send};

/// A provider that lists one model and answers "Hello!" in two deltas, with usage.
async fn provider() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer sk-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [{ "id": "mock-1", "object": "model", "created": 1, "owned_by": "test" }],
        })))
        .mount(&server)
        .await;
    let chunk = |delta: Value, finish: Value| {
        json!({
            "id": "chatcmpl-1",
            "object": "chat.completion.chunk",
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
        })
    };
    let events = [
        chunk(
            json!({ "role": "assistant", "content": "Hel" }),
            Value::Null,
        ),
        chunk(json!({ "content": "lo!" }), Value::Null),
        chunk(json!({}), json!("stop")),
        json!({
            "id": "chatcmpl-1",
            "object": "chat.completion.chunk",
            "choices": [],
            "usage": { "prompt_tokens": 12, "completion_tokens": 2, "total_tokens": 14 },
        }),
    ];
    let mut body: String = events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect();
    body.push_str("data: [DONE]\n\n");
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-test"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;
    server
}

/// `POST`, `PATCH`, or `GET` with the caller's token; the JSON body of the answer.
async fn call(app: &Router, token: &str, method: Method, uri: &str, body: Option<Value>) -> Value {
    let response = send(app.clone(), request(method.clone(), uri, Some(token), body)).await;
    assert!(
        response.status.is_success(),
        "{method} {uri}: {} {}",
        response.status,
        response.body
    );
    response.body
}

/// A connection to `server`, refreshed, with its one model enabled; the model's id.
async fn connect(app: &Router, token: &str, server: &MockServer) -> String {
    let created = call(
        app,
        token,
        Method::POST,
        "/api/providers",
        Some(json!({
            "kind": "openai",
            "preset": "custom",
            "name": "Mock",
            "baseUrl": format!("{}/v1", server.uri()),
            "apiKey": "sk-test",
        })),
    )
    .await;
    assert_eq!(created["data"]["key"], "set");
    let provider = created["data"]["id"].as_str().expect("an id").to_owned();

    let refreshed = call(
        app,
        token,
        Method::POST,
        &format!("/api/providers/{provider}/refresh"),
        None,
    )
    .await;
    assert_eq!(
        refreshed["data"],
        json!({ "added": 1, "updated": 0, "unavailable": 0, "enabled": 1 })
    );
    let models = call(
        app,
        token,
        Method::GET,
        &format!("/api/providers/{provider}/models"),
        None,
    )
    .await;
    assert_eq!(models["total"], 1);
    let model = models["items"][0]["id"].as_str().expect("an id").to_owned();

    let enabled = call(
        app,
        token,
        Method::PATCH,
        &format!("/api/models/{model}"),
        Some(json!({ "enabled": true })),
    )
    .await;
    assert_eq!(enabled["data"]["enabled"], true);
    assert_eq!(enabled["data"]["remoteId"], "mock-1");
    let picker = call(app, token, Method::GET, "/api/models", None).await;
    assert_eq!(picker["items"][0]["id"], model.as_str());
    model
}

/// The `data` of every event of a stream body.
fn events(body: &str) -> Vec<Value> {
    body.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).expect("JSON event data"))
        .collect()
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn a_message_streams_a_reply_that_is_saved(pool: DbPool) {
    let server = provider().await;
    let app = live_app(pool).await;
    let token = register(&app, "admin").await;
    let model = connect(&app, &token, &server).await;

    let chat = call(
        &app,
        &token,
        Method::POST,
        "/api/chats",
        Some(json!({ "modelId": model })),
    )
    .await;
    let chat = chat["data"]["id"].as_str().expect("an id").to_owned();

    let (status, headers, body) = read(
        app.clone(),
        request(
            Method::POST,
            &format!("/api/chats/{chat}/messages"),
            Some(&token),
            Some(json!({ "text": "Say hello" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(headers[CONTENT_TYPE], "text/event-stream");
    let events = events(&body);
    let types: Vec<&str> = events
        .iter()
        .map(|event| event["type"].as_str().expect("a type"))
        .collect();
    assert_eq!(types.first(), Some(&"message.started"));
    assert_eq!(types.last(), Some(&"done"), "{body}");
    let seqs: Vec<u64> = events
        .iter()
        .map(|event| event["seq"].as_u64().expect("a seq"))
        .collect();
    let expected: Vec<u64> = (1..=seqs.len() as u64).collect();
    assert_eq!(seqs, expected, "numbered from 1 without gaps");
    let streamed: String = events
        .iter()
        .filter(|event| event["type"] == "text.delta")
        .map(|event| event["text"].as_str().expect("text"))
        .collect();
    assert_eq!(streamed, "Hello!");
    let finished = events
        .iter()
        .find(|event| event["type"] == "message.finished")
        .expect("message.finished");
    assert_eq!(finished["status"], "complete");
    assert_eq!(finished["finishReason"], "stop");
    assert_eq!(finished["usage"]["inputTokens"], 12);
    assert_eq!(finished["messageId"], events[0]["messageId"]);

    // The provider got the user's text, with the key in the header only.
    let sent = server.received_requests().await.expect("recording");
    let completion = sent
        .iter()
        .find(|request| request.url.path() == "/v1/chat/completions")
        .expect("a completion request");
    let request_body: Value = serde_json::from_slice(&completion.body).expect("JSON");
    assert_eq!(request_body["model"], "mock-1");
    assert_eq!(request_body["stream"], true);
    let last = request_body["messages"]
        .as_array()
        .and_then(|messages| messages.last())
        .expect("messages");
    assert_eq!(last["role"], "user");
    assert!(last["content"].to_string().contains("Say hello"));

    // Saved: the user message and the complete answer, and the chat is titled.
    let saved = call(
        &app,
        &token,
        Method::GET,
        &format!("/api/chats/{chat}/messages"),
        None,
    )
    .await;
    assert_eq!(saved["total"], 2, "{saved}");
    let [question, answer] = [&saved["items"][0], &saved["items"][1]];
    assert_eq!(question["role"], "user");
    assert_eq!(
        question["parts"],
        json!([{ "type": "text", "text": "Say hello" }])
    );
    assert_eq!(answer["role"], "assistant");
    assert_eq!(answer["status"], "complete");
    assert_eq!(answer["finishReason"], "stop");
    assert_eq!(answer["modelId"], model.as_str());
    assert_eq!(
        answer["parts"],
        json!([{ "type": "text", "text": "Hello!" }])
    );
    assert_eq!(answer["usage"]["outputTokens"], 2);
    assert_eq!(answer["id"], events[0]["messageId"]);
    let details = call(
        &app,
        &token,
        Method::GET,
        &format!("/api/chats/{chat}"),
        None,
    )
    .await;
    assert_eq!(details["data"]["title"], "Say hello");

    // The slot is free again: a second message answers too.
    let (status, _, body) = read(
        app.clone(),
        request(
            Method::POST,
            &format!("/api/chats/{chat}/messages"),
            Some(&token),
            Some(json!({ "text": "Again" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let saved = call(
        &app,
        &token,
        Method::GET,
        &format!("/api/chats/{chat}/messages"),
        None,
    )
    .await;
    assert_eq!(saved["total"], 4);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn a_failed_turn_changes_nothing_and_frees_the_chat(pool: DbPool) {
    let app = live_app(pool).await;
    let token = register(&app, "admin").await;
    let chat = call(&app, &token, Method::POST, "/api/chats", Some(json!({}))).await;
    let chat = chat["data"]["id"].as_str().expect("an id").to_owned();
    let uri = format!("/api/chats/{chat}/messages");

    for _ in 0..2 {
        let response = send(
            app.clone(),
            request(
                Method::POST,
                &uri,
                Some(&token),
                Some(json!({ "text": "Hello" })),
            ),
        )
        .await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "never CHAT_BUSY");
        assert_eq!(response.body["error"], "Choose a model for this chat.");
        assert_eq!(response.body["field"], "modelId");
    }
    let saved = call(&app, &token, Method::GET, &uri, None).await;
    assert_eq!(saved["total"], 0, "nothing was written");

    let unknown = send(
        app.clone(),
        request(
            Method::POST,
            "/api/chats/0196f6a4-7c1e-7d2a-9b3c-4d5e6f708192/messages",
            Some(&token),
            Some(json!({ "text": "Hello" })),
        ),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn a_provider_failure_ends_the_stream_with_a_saved_error(pool: DbPool) {
    let server = provider().await;
    let app = live_app(pool).await;
    let token = register(&app, "admin").await;
    let model = connect(&app, &token, &server).await;
    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_string("bad key sk-test"))
        .mount(&server)
        .await;
    let chat = call(
        &app,
        &token,
        Method::POST,
        "/api/chats",
        Some(json!({ "modelId": model })),
    )
    .await;
    let chat = chat["data"]["id"].as_str().expect("an id").to_owned();

    let (status, _, body) = read(
        app.clone(),
        request(
            Method::POST,
            &format!("/api/chats/{chat}/messages"),
            Some(&token),
            Some(json!({ "text": "Hello" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the stream had started");
    assert!(
        !body.contains("sk-test"),
        "never echoes the provider's body"
    );
    let events = events(&body);
    let finished = events
        .iter()
        .find(|event| event["type"] == "message.finished")
        .expect("message.finished");
    assert_eq!(finished["status"], "error");
    let error = events
        .iter()
        .find(|event| event["type"] == "error")
        .expect("an error event");
    assert_eq!(error["code"], "PROVIDER_AUTH");
    assert_eq!(events.last().expect("events")["type"], "done");

    let saved = call(
        &app,
        &token,
        Method::GET,
        &format!("/api/chats/{chat}/messages"),
        None,
    )
    .await;
    assert_eq!(saved["items"][1]["status"], "error");
    assert_eq!(saved["items"][1]["error"], error["message"]);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn turning_a_model_off_answers_with_the_saved_model(pool: DbPool) {
    let server = provider().await;
    let app = live_app(pool).await;
    let token = register(&app, "admin").await;
    let model = connect(&app, &token, &server).await;

    let response = send(
        app.clone(),
        request(
            Method::PATCH,
            &format!("/api/models/{model}"),
            Some(&token),
            Some(json!({ "enabled": false })),
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(response.body["data"]["enabled"], false);
    let picker = call(&app, &token, Method::GET, "/api/models", None).await;
    assert_eq!(picker["total"], 0, "off models leave the picker");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn a_chat_in_an_unknown_project_is_not_found(pool: DbPool) {
    let app = live_app(pool).await;
    let token = register(&app, "admin").await;

    let response = send(
        app.clone(),
        request(
            Method::POST,
            "/api/chats",
            Some(&token),
            Some(json!({ "projectId": "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708192" })),
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND, "{}", response.body);
    assert_eq!(response.body["code"], "ENTITY_NOT_FOUND");
}
