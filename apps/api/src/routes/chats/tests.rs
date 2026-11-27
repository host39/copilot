//! The walking skeleton end to end, then the chat list, settings, read state, deletes, and save
//! as project (`make test-db`): rustauth and Copilot on a throwaway Postgres database, and a
//! wiremock server as the OpenAI-compatible provider, reached through the real adapter and the
//! hardened clients. The first account is an administrator, so its connection may use the
//! loopback address of the mock (`ALLOW_PRIVATE_NETWORK=admins`).

use std::time::{Duration, Instant};

use axum::Router;
use axum::http::header::CONTENT_TYPE;
use axum::http::{Method, StatusCode};
use copilot_database::DbPool;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::helpers::testing::{
    self, TestResponse, live_app, other_user, read, register, request, send, tool_server,
};

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

/// Any answer, as status and body.
async fn answer(
    app: &Router,
    token: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> TestResponse {
    send(app.clone(), request(method, uri, Some(token), body)).await
}

/// A new chat of the caller with `body`; its id.
async fn new_chat(app: &Router, token: &str, body: Value) -> String {
    let chat = call(app, token, Method::POST, "/api/chats", Some(body)).await;
    chat["data"]["id"].as_str().expect("an id").to_owned()
}

/// Sends `text` in `chat` and reads the stream to its end.
async fn say(app: &Router, token: &str, chat: &str, text: &str) {
    let (status, _, body) = read(
        app.clone(),
        request(
            Method::POST,
            &format!("/api/chats/{chat}/messages"),
            Some(token),
            Some(json!({ "text": text })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"type\":\"done\""), "{body}");
}

/// The ids of a page of `GET /chats`.
fn ids(page: &Value) -> Vec<String> {
    page["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|row| row["id"].as_str().expect("an id").to_owned())
        .collect()
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn chats_are_listed_changed_read_and_deleted(pool: DbPool) {
    let server = provider().await;
    let app = live_app(pool.clone()).await;
    let token = register(&app, "admin").await;
    let model = connect(&app, &token, &server).await;
    let docs = tool_server(&pool, "admin", "docs").await;
    let project = call(
        &app,
        &token,
        Method::POST,
        "/api/projects",
        Some(json!({ "name": "Research" })),
    )
    .await;
    let project = project["data"]["id"].as_str().expect("an id").to_owned();

    // Oldest first: an empty chat, then two chats that answered.
    let third = new_chat(&app, &token, json!({})).await;
    let first = new_chat(&app, &token, json!({ "modelId": model })).await;
    say(&app, &token, &first, "What is Rust?").await;
    let second = new_chat(
        &app,
        &token,
        json!({ "projectId": project, "modelId": model }),
    )
    .await;
    say(&app, &token, &second, "Plan the week").await;

    let all = call(&app, &token, Method::GET, "/api/chats", None).await;
    assert_eq!(
        ids(&all),
        vec![second.clone(), first.clone(), third.clone()]
    );
    assert_eq!(all["next"], Value::Null);
    let row = &all["items"][0];
    assert_eq!(row["title"], "Plan the week");
    assert_eq!(row["projectId"], project.as_str());
    assert_eq!(row["modelId"], model.as_str());
    assert_eq!(row["scheduleId"], Value::Null);
    assert_eq!(row["unread"], true, "answered after it was opened");
    assert_eq!(row["awaitingApproval"], false);
    assert_eq!(all["items"][2]["unread"], false, "nothing happened since");

    // One chat per page, through the cursor.
    let mut seen = Vec::new();
    let mut uri = "/api/chats?limit=1".to_owned();
    loop {
        let page = call(&app, &token, Method::GET, &uri, None).await;
        assert_eq!(page["items"].as_array().expect("items").len(), 1);
        seen.extend(ids(&page));
        match page["next"].as_str() {
            Some(next) => uri = format!("/api/chats?limit=1&cursor={next}"),
            None => break,
        }
    }
    assert_eq!(seen, ids(&all));

    let in_project = call(
        &app,
        &token,
        Method::GET,
        &format!("/api/chats?projectId={project}&scheduled=false"),
        None,
    )
    .await;
    assert_eq!(ids(&in_project), vec![second.clone()]);
    let runs = call(&app, &token, Method::GET, "/api/chats?scheduled=true", None).await;
    assert!(ids(&runs).is_empty(), "no schedule runs yet");
    for (uri, field) in [
        ("/api/chats?cursor=not-a-cursor", Some("cursor")),
        ("/api/chats?pinned=maybe", None),
    ] {
        let invalid = answer(&app, &token, Method::GET, uri, None).await;
        assert_eq!(invalid.status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(invalid.body["code"], "VALIDATION_ERROR");
        assert_eq!(invalid.body["field"].as_str(), field, "{uri}");
    }
    let unknown = answer(
        &app,
        &token,
        Method::GET,
        "/api/chats?projectId=0196f6a4-7c1e-7d2a-9b3c-4d5e6f708192",
        None,
    )
    .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);

    let uri = format!("/api/chats/{first}");
    let changed = call(
        &app,
        &token,
        Method::PATCH,
        &uri,
        Some(json!({
            "title": "  Rust notes ",
            "projectId": project,
            "reasoning": "high",
            "builtinTools": ["schedules"],
            "toolOverrides": [{ "serverId": docs, "enabled": false }],
            "pinned": true,
            "voiceReplies": true,
        })),
    )
    .await;
    let data = &changed["data"];
    assert_eq!(data["title"], "Rust notes");
    assert_eq!(data["projectId"], project.as_str());
    assert_eq!(data["reasoning"], "high");
    assert_eq!(data["builtinTools"], json!(["schedules"]));
    assert_eq!(
        data["toolOverrides"],
        json!([{ "serverId": docs, "enabled": false }])
    );
    assert_eq!(data["pinned"], true);
    assert_eq!(data["voiceReplies"], true);
    assert_eq!(data["modelId"], model.as_str(), "fields left out stay");
    let cleared = call(
        &app,
        &token,
        Method::PATCH,
        &uri,
        Some(json!({
            "projectId": null,
            "modelId": null,
            "reasoning": null,
            "builtinTools": null,
            "toolOverrides": [],
        })),
    )
    .await;
    let data = &cleared["data"];
    assert_eq!(data["projectId"], Value::Null);
    assert_eq!(data["modelId"], Value::Null);
    assert_eq!(data["reasoning"], Value::Null);
    assert_eq!(data["builtinTools"], Value::Null);
    assert_eq!(data["toolOverrides"], json!([]));
    assert_eq!(data["title"], "Rust notes");
    let blank = answer(
        &app,
        &token,
        Method::PATCH,
        &uri,
        Some(json!({ "title": " " })),
    )
    .await;
    assert_eq!(blank.status, StatusCode::BAD_REQUEST);
    let pinned = call(&app, &token, Method::GET, "/api/chats?pinned=true", None).await;
    assert_eq!(ids(&pinned), vec![first.clone()]);
    let after = call(&app, &token, Method::GET, "/api/chats", None).await;
    assert_eq!(
        ids(&after),
        ids(&all),
        "settings changes do not move a chat in the list"
    );

    let read_at = answer(
        &app,
        &token,
        Method::POST,
        &format!("/api/chats/{second}/read"),
        None,
    )
    .await;
    assert_eq!(read_at.status, StatusCode::NO_CONTENT);
    let after = call(&app, &token, Method::GET, "/api/chats", None).await;
    assert_eq!(after["items"][0]["id"], second.as_str());
    assert_eq!(after["items"][0]["unread"], false);
    let details = call(
        &app,
        &token,
        Method::GET,
        &format!("/api/chats/{second}"),
        None,
    )
    .await;
    assert!(details["data"]["lastReadAt"].is_string());

    let deleted = answer(&app, &token, Method::DELETE, &uri, None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    for method in [Method::GET, Method::DELETE] {
        let gone = answer(&app, &token, method.clone(), &uri, None).await;
        assert_eq!(gone.status, StatusCode::NOT_FOUND, "{method}");
    }
    let after = call(&app, &token, Method::GET, "/api/chats", None).await;
    assert_eq!(ids(&after), vec![second, third]);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn another_user_gets_404_for_chats_and_for_ids_in_bodies(pool: DbPool) {
    let server = provider().await;
    let app = live_app(pool.clone()).await;
    let alice = register(&app, "alice").await;
    let bob = other_user(&app, &alice, "bob").await;
    let model = connect(&app, &alice, &server).await;
    let docs = tool_server(&pool, "alice", "docs").await;
    let project = call(
        &app,
        &alice,
        Method::POST,
        "/api/projects",
        Some(json!({ "name": "Private" })),
    )
    .await;
    let project = project["data"]["id"].as_str().expect("an id").to_owned();
    let chat = new_chat(
        &app,
        &alice,
        json!({ "projectId": project, "modelId": model, "instructions": "Secret plans." }),
    )
    .await;
    say(&app, &alice, &chat, "Hello").await;

    let uri = format!("/api/chats/{chat}");
    for (method, path, body) in [
        (Method::GET, uri.clone(), None),
        (
            Method::PATCH,
            uri.clone(),
            Some(json!({ "title": "Taken" })),
        ),
        (Method::DELETE, uri.clone(), None),
        (Method::POST, format!("{uri}/read"), None),
        (Method::GET, format!("{uri}/messages"), None),
        (Method::POST, format!("{uri}/save-as-project"), None),
        (Method::POST, format!("{uri}/save-instructions"), None),
        (
            Method::POST,
            format!("{uri}/messages"),
            Some(json!({ "text": "Hi" })),
        ),
    ] {
        let response = answer(&app, &bob, method.clone(), &path, body).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND, "{method} {path}");
        assert_eq!(response.body["code"], "ENTITY_NOT_FOUND");
    }
    let list = call(&app, &bob, Method::GET, "/api/chats", None).await;
    assert!(ids(&list).is_empty());
    let filtered = answer(
        &app,
        &bob,
        Method::GET,
        &format!("/api/chats?projectId={project}"),
        None,
    )
    .await;
    assert_eq!(
        filtered.status,
        StatusCode::NOT_FOUND,
        "A's project as a filter"
    );

    let own = new_chat(&app, &bob, json!({})).await;
    let own = format!("/api/chats/{own}");
    for body in [
        json!({ "projectId": project }),
        json!({ "modelId": model }),
        json!({ "toolOverrides": [{ "serverId": docs, "enabled": true }] }),
    ] {
        let response = answer(&app, &bob, Method::PATCH, &own, Some(body.clone())).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND, "{body}");
    }
    let created = answer(
        &app,
        &bob,
        Method::POST,
        "/api/chats",
        Some(json!({ "toolOverrides": [{ "serverId": docs, "enabled": true }] })),
    )
    .await;
    assert_eq!(
        created.status,
        StatusCode::NOT_FOUND,
        "A's server in a new chat"
    );

    let mine = call(&app, &alice, Method::GET, &uri, None).await;
    assert_eq!(mine["data"]["title"], "Hello", "unchanged");
    assert_eq!(mine["data"]["instructions"], "Secret plans.");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn save_as_project_moves_the_chat_and_save_instructions_appends_them(pool: DbPool) {
    let server = provider().await;
    let app = live_app(pool.clone()).await;
    let token = register(&app, "admin").await;
    let model = connect(&app, &token, &server).await;
    let docs = tool_server(&pool, "admin", "docs").await;
    let chat = new_chat(
        &app,
        &token,
        json!({
            "modelId": model,
            "instructions": "Answer in French.",
            "reasoning": "low",
            "toolOverrides": [{ "serverId": docs, "enabled": true }],
        }),
    )
    .await;
    say(&app, &token, &chat, "Trip ideas").await;
    let uri = format!("/api/chats/{chat}");

    let saved = call(
        &app,
        &token,
        Method::POST,
        &format!("{uri}/save-as-project"),
        None,
    )
    .await;
    let project = &saved["data"];
    assert_eq!(project["name"], "Trip ideas");
    assert_eq!(project["instructions"], "Answer in French.");
    assert_eq!(project["defaultModelId"], model.as_str());
    assert_eq!(project["reasoning"], "low");
    assert_eq!(project["toolServerIds"], json!([docs]));
    let project_id = project["id"].as_str().expect("an id").to_owned();
    let moved = call(&app, &token, Method::GET, &uri, None).await;
    assert_eq!(moved["data"]["projectId"], project_id.as_str());
    assert_eq!(moved["data"]["instructions"], "", "nothing is sent twice");
    assert_eq!(moved["data"]["toolOverrides"], json!([]));
    let again = answer(
        &app,
        &token,
        Method::POST,
        &format!("{uri}/save-as-project"),
        None,
    )
    .await;
    assert_eq!(again.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(again.body["code"], "CHAT_IN_PROJECT");

    let empty = answer(
        &app,
        &token,
        Method::POST,
        &format!("{uri}/save-instructions"),
        None,
    )
    .await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST);
    call(
        &app,
        &token,
        Method::PATCH,
        &uri,
        Some(json!({ "instructions": "Keep it short." })),
    )
    .await;
    let appended = call(
        &app,
        &token,
        Method::POST,
        &format!("{uri}/save-instructions"),
        None,
    )
    .await;
    assert_eq!(
        appended["data"]["instructions"],
        "Answer in French.\n\nKeep it short."
    );
    let chat = call(&app, &token, Method::GET, &uri, None).await;
    assert_eq!(chat["data"]["instructions"], "");

    let standalone = new_chat(&app, &token, json!({ "instructions": "Hi." })).await;
    let outside = answer(
        &app,
        &token,
        Method::POST,
        &format!("/api/chats/{standalone}/save-instructions"),
        None,
    )
    .await;
    assert_eq!(outside.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(outside.body["code"], "CHAT_NOT_IN_PROJECT");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn an_edit_replaces_the_message_and_everything_after_it(pool: DbPool) {
    let server = provider().await;
    let app = live_app(pool).await;
    let token = register(&app, "admin").await;
    let model = connect(&app, &token, &server).await;
    let chat = new_chat(&app, &token, json!({ "modelId": model })).await;
    say(&app, &token, &chat, "First question").await;
    say(&app, &token, &chat, "Second question").await;
    let uri = format!("/api/chats/{chat}/messages");
    let before = call(&app, &token, Method::GET, &uri, None).await;
    assert_eq!(before["total"], 4);
    let first = before["items"][0]["id"].as_str().expect("an id").to_owned();

    let (status, _, body) = read(
        app.clone(),
        request(
            Method::POST,
            &uri,
            Some(&token),
            Some(json!({ "text": "Edited question", "editFrom": first })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let after = call(&app, &token, Method::GET, &uri, None).await;
    assert_eq!(after["total"], 2, "the edit and its answer: {after}");
    assert_eq!(after["items"][0]["parts"][0]["text"], "Edited question");
    assert_eq!(after["items"][1]["role"], "assistant");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn deleting_a_chat_stops_its_live_run_first(pool: DbPool) {
    let server = testing::provider().await;
    // A provider that takes long to answer keeps the run live.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(60)))
        .mount(&server)
        .await;
    let app = live_app(pool).await;
    let token = register(&app, "admin").await;
    let model = testing::model(&app, &token, &server).await;
    let chat = new_chat(&app, &token, json!({ "modelId": model })).await;

    let turn = tokio::spawn(read(
        app.clone(),
        request(
            Method::POST,
            &format!("/api/chats/{chat}/messages"),
            Some(&token),
            Some(json!({ "text": "Hello" })),
        ),
    ));
    let started = Instant::now();
    loop {
        let requests = server.received_requests().await.unwrap_or_default();
        if requests
            .iter()
            .any(|request| request.url.path() == "/v1/chat/completions")
        {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the run starts"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let deleted = answer(
        &app,
        &token,
        Method::DELETE,
        &format!("/api/chats/{chat}"),
        None,
    )
    .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.body);
    let (status, _, body) = tokio::time::timeout(Duration::from_secs(10), turn)
        .await
        .expect("the run ended with the delete, long before the provider answered")
        .expect("the turn task");
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"cancelled\""), "{body}");
    assert!(body.contains("\"type\":\"done\""), "{body}");
    let gone = answer(
        &app,
        &token,
        Method::GET,
        &format!("/api/chats/{chat}"),
        None,
    )
    .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    let list = call(&app, &token, Method::GET, "/api/chats", None).await;
    assert!(
        ids(&list).is_empty(),
        "the cancelled answer did not bring it back"
    );
}
