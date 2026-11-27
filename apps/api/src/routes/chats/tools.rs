//! Tool calls in real turns, end to end (`make test-db`): wiremock as the provider, the
//! in-process rmcp server ([`McpServer`]) as the tool server, and the real MCP adapter in
//! between. An `auto` tool runs and the answer goes on; an `ask` tool pauses the run, and the
//! decisions route approves it (once, or for the chat) or denies it with a reason; a new message
//! while a call waits skips it; and `GET /api/events` tells the owner about each run.

use std::time::Duration;

use axum::Router;
use axum::http::{Method, StatusCode};
use copilot_database::DbPool;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::helpers::mcp_server::McpServer;
use crate::helpers::testing::{self, Events, open, other_user, request, send};
use crate::routes::chats::flows::{call, content, messages, say, setup, text, turn};

/// A completion stream of `chunks` (`delta` and `finish_reason` of each).
fn stream(chunks: &[(Value, Value)]) -> String {
    let mut body: String = chunks
        .iter()
        .map(|(delta, finish)| {
            let chunk = json!({
                "id": "chatcmpl-1",
                "object": "chat.completion.chunk",
                "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
            });
            format!("data: {chunk}\n\n")
        })
        .collect();
    body.push_str("data: [DONE]\n\n");
    body
}

/// An answer of `text` that stops.
fn answer(text: &str) -> String {
    stream(&[
        (json!({ "content": text }), Value::Null),
        (json!({}), json!("stop")),
    ])
}

/// An answer that calls each `(alias, arguments)` in order.
fn calls(calls: &[(&str, &str)]) -> String {
    let mut chunks: Vec<(Value, Value)> = calls
        .iter()
        .enumerate()
        .map(|(index, (alias, arguments))| {
            let delta = json!({ "tool_calls": [{
                "index": index,
                "id": format!("call_{index}"),
                "type": "function",
                "function": { "name": alias, "arguments": arguments },
            }] });
            (delta, Value::Null)
        })
        .collect();
    chunks.push((json!({}), json!("tool_calls")));
    stream(&chunks)
}

/// The next completion (after the ones scripted before) answers `body`, after `delay`.
async fn next(provider: &MockServer, body: String, delay: Duration) {
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body)
                .set_delay(delay),
        )
        .up_to_n_times(1)
        .mount(provider)
        .await;
}

async fn script(provider: &MockServer, body: String) {
    next(provider, body, Duration::ZERO).await;
}

/// The completion request bodies the provider received, in order.
async fn completions(provider: &MockServer) -> Vec<Value> {
    provider
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| request.url.path() == "/v1/chat/completions")
        .map(|request| serde_json::from_slice(&request.body).expect("a JSON body"))
        .collect()
}

/// The app with a wiremock provider: the app, the first account's token, the model's id, and
/// the provider to script.
async fn start(pool: DbPool) -> (Router, String, String, MockServer) {
    let provider = testing::provider().await;
    let (app, token, model) = setup(pool, &format!("{}/v1", provider.uri())).await;
    (app.router, token, model, provider)
}

/// The tool server "Docs" (slug `docs`, so the aliases are `docs__search` and `docs__delete`)
/// on an in-process rmcp server of the 2026-07-28 revision, with the approval mode `approval`;
/// the server and its id.
async fn docs(app: &Router, token: &str, approval: &str) -> (McpServer, String) {
    docs_on(app, token, approval, false).await
}

/// [`docs`] on a server of the 2026-07-28 revision, or of 2025-11-25 and older (`legacy`).
async fn docs_on(app: &Router, token: &str, approval: &str, legacy: bool) -> (McpServer, String) {
    let mcp = McpServer::start(
        legacy,
        &[
            (
                "search".to_owned(),
                "Search the docs.".to_owned(),
                json!({ "type": "object", "properties": { "q": { "type": "string" } } }),
            ),
            (
                "delete".to_owned(),
                "Delete a page.".to_owned(),
                json!({ "type": "object", "properties": { "page": { "type": "string" } } }),
            ),
        ],
        None,
    )
    .await;
    let created = call(
        app,
        token,
        Method::POST,
        "/api/tool-servers",
        Some(json!({ "name": "Docs", "url": mcp.url, "approval": approval })),
    )
    .await;
    let id = created["data"]["id"].as_str().expect("an id").to_owned();
    (mcp, id)
}

/// A chat with the tool server `server` turned on.
async fn tool_chat(app: &Router, token: &str, model: &str, server: &str) -> String {
    let chat = call(
        app,
        token,
        Method::POST,
        "/api/chats",
        Some(json!({
            "modelId": model,
            "toolOverrides": [{ "serverId": server, "enabled": true }],
        })),
    )
    .await;
    chat["data"]["id"].as_str().expect("an id").to_owned()
}

/// The events of `kind`.
fn all<'a>(events: &'a [Value], kind: &str) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|event| event["type"] == kind)
        .collect()
}

/// The only event of `kind`.
fn one<'a>(events: &'a [Value], kind: &str) -> &'a Value {
    let found = all(events, kind);
    assert_eq!(found.len(), 1, "one {kind}: {events:?}");
    found[0]
}

fn roles(saved: &[Value]) -> Vec<&str> {
    saved
        .iter()
        .map(|message| message["role"].as_str().expect("a role"))
        .collect()
}

/// Sends `text` to a chat whose answer calls a tool that asks: the paused message's id and the
/// call's id.
async fn pause(app: &Router, token: &str, chat: &str, text: &str) -> (String, String) {
    let events = say(app, token, chat, text).await;
    let requested = one(&events, "tool.requested");
    assert_eq!(requested["needsApproval"], true);
    assert!(all(&events, "tool.finished").is_empty(), "nothing ran");
    let finishes = all(&events, "message.finished");
    let finished = finishes.last().expect("a message.finished");
    assert_eq!(finished["status"], "awaiting_approval");
    (
        finished["messageId"].as_str().expect("an id").to_owned(),
        requested["callId"].as_str().expect("an id").to_owned(),
    )
}

/// Posts `decisions` on `message` and reads the continuation to its end.
async fn decide(
    app: &Router,
    token: &str,
    chat: &str,
    message: &str,
    decisions: Value,
) -> Vec<Value> {
    let route = format!("messages/{message}/decisions");
    let mut events = turn(
        app,
        token,
        chat,
        &route,
        Some(json!({ "decisions": decisions })),
    )
    .await;
    let all = events.rest().await;
    assert_eq!(all.last().map(|event| &event["type"]), Some(&json!("done")));
    all
}

/// `POST /chats/{chat}/messages/{message}/decisions` answered without a stream.
async fn refused(
    app: &Router,
    token: &str,
    chat: &str,
    message: &str,
    decisions: Value,
) -> testing::TestResponse {
    send(
        app.clone(),
        request(
            Method::POST,
            &format!("/api/chats/{chat}/messages/{message}/decisions"),
            Some(token),
            Some(json!({ "decisions": decisions })),
        ),
    )
    .await
}

/// The text of a tool message sent to the provider for `call_id` (the provider's call id).
fn sent_result<'a>(request: &'a Value, call_id: &str) -> &'a Value {
    request["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .find(|message| message["role"] == "tool" && message["tool_call_id"] == call_id)
        .map(|message| &message["content"])
        .unwrap_or_else(|| panic!("a result for {call_id}: {request}"))
}

/// The id of the first call in the last assistant message of a request.
fn sent_call(request: &Value) -> String {
    let assistant = request["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .rev()
        .find(|message| message["role"] == "assistant")
        .expect("an assistant message");
    assistant["tool_calls"][0]["id"]
        .as_str()
        .expect("a call id")
        .to_owned()
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn an_auto_tool_runs_on_the_tool_server_and_the_answer_goes_on(pool: DbPool) {
    let (app, token, model, provider) = start(pool).await;
    let (mcp, server) = docs(&app, &token, "auto").await;
    let chat = tool_chat(&app, &token, &model, &server).await;
    script(&provider, calls(&[("docs__search", r#"{"q":"rust"}"#)])).await;
    script(&provider, answer("Found it.")).await;

    let events = say(&app, &token, &chat, "Look it up").await;
    let requested = one(&events, "tool.requested");
    assert_eq!(requested["toolName"], "search", "the name, never the alias");
    assert_eq!(requested["serverId"], server.as_str());
    assert_eq!(requested["needsApproval"], false);
    assert_eq!(requested["arguments"], json!({ "q": "rust" }));
    let finished = one(&events, "tool.finished");
    assert_eq!(finished["callId"], requested["callId"]);
    assert_eq!(finished["status"], "ok");
    assert_eq!(
        finished["output"],
        json!([{ "type": "text", "text": r#"search {"q":"rust"}"# }])
    );
    assert_eq!(text(&events), "Found it.");
    assert_eq!(
        mcp.calls(),
        [("search".to_owned(), json!({ "q": "rust" }))],
        "the stored arguments, once"
    );

    let saved = messages(&app, &token, &chat).await;
    assert_eq!(roles(&saved), ["user", "assistant", "tool", "assistant"]);
    assert_eq!(saved[1]["finishReason"], "tool_calls");
    assert_eq!(saved[1]["parts"][0]["approval"], "auto");
    assert_eq!(saved[2]["parts"][0]["status"], "ok");
    assert_eq!(saved[3]["status"], "complete");
    assert_eq!(content(&saved[3]), "Found it.");

    let sent = completions(&provider).await;
    assert_eq!(sent.len(), 2);
    let offered: Vec<&str> = sent[0]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|tool| tool["function"]["name"].as_str().expect("a name"))
        .collect();
    assert_eq!(offered, ["docs__delete", "docs__search"]);
    let call_id = sent_call(&sent[1]);
    assert_eq!(sent_result(&sent[1], &call_id), r#"search {"q":"rust"}"#);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn a_tool_server_of_the_older_era_runs_tools_in_turns_too(pool: DbPool) {
    let (app, token, model, provider) = start(pool).await;
    let (mcp, server) = docs_on(&app, &token, "auto", true).await;
    let chat = tool_chat(&app, &token, &model, &server).await;
    script(&provider, calls(&[("docs__search", r#"{"q":"old"}"#)])).await;
    script(&provider, answer("Found.")).await;
    let events = say(&app, &token, &chat, "Look it up").await;
    assert_eq!(one(&events, "tool.finished")["status"], "ok");
    assert_eq!(text(&events), "Found.");
    assert_eq!(mcp.calls(), [("search".to_owned(), json!({ "q": "old" }))]);
    let details = call(
        &app,
        &token,
        Method::GET,
        &format!("/api/tool-servers/{server}"),
        None,
    )
    .await;
    assert_eq!(
        details["data"]["protocolVersion"], "2025-11-25",
        "the turn listed the server"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn an_ask_tool_pauses_and_an_approval_continues_the_turn(pool: DbPool) {
    let (app, token, model, provider) = start(pool).await;
    let (mcp, server) = docs(&app, &token, "ask").await;
    let chat = tool_chat(&app, &token, &model, &server).await;
    script(&provider, calls(&[("docs__delete", r#"{"page":"intro"}"#)])).await;

    let (paused, call_id) = pause(&app, &token, &chat, "Delete the intro").await;
    assert!(mcp.calls().is_empty(), "nothing runs before a decision");
    let saved = messages(&app, &token, &chat).await;
    assert_eq!(roles(&saved), ["user", "assistant"]);
    assert_eq!(saved[1]["status"], "awaiting_approval");
    assert_eq!(saved[1]["parts"][0]["approval"], "pending");
    let list = call(&app, &token, Method::GET, "/api/chats", None).await;
    assert_eq!(list["items"][0]["awaitingApproval"], true);

    // The decisions must cover exactly the pending calls, on the paused message.
    let none = refused(&app, &token, &chat, &paused, json!([])).await;
    assert_eq!(none.status, StatusCode::BAD_REQUEST, "{}", none.body);
    assert_eq!(none.body["field"], "decisions");
    let twice = json!([
        { "callId": call_id, "decision": "approve" },
        { "callId": call_id, "decision": "deny" },
    ]);
    let twice = refused(&app, &token, &chat, &paused, twice).await;
    assert_eq!(twice.status, StatusCode::BAD_REQUEST, "{}", twice.body);
    let approve = json!([{ "callId": call_id, "decision": "approve" }]);
    let user = saved[0]["id"].as_str().expect("an id");
    let wrong = refused(&app, &token, &chat, user, approve.clone()).await;
    assert_eq!(
        wrong.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        wrong.body
    );
    assert_eq!(wrong.body["code"], "NOT_AWAITING_APPROVAL");
    let unknown = refused(
        &app,
        &token,
        &chat,
        &uuid::Uuid::now_v7().to_string(),
        approve.clone(),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND, "{}", unknown.body);
    let malformed = refused(&app, &token, &chat, "not-a-uuid", approve.clone()).await;
    assert_eq!(malformed.status, StatusCode::BAD_REQUEST);
    assert_eq!(malformed.body["field"], "messageId");
    let other = other_user(&app, &token, "bob").await;
    let foreign = refused(&app, &other, &chat, &paused, approve.clone()).await;
    assert_eq!(foreign.status, StatusCode::NOT_FOUND, "another user's chat");
    assert!(mcp.calls().is_empty(), "no refused decision ran anything");

    script(&provider, answer("Deleted.")).await;
    let events = decide(&app, &token, &chat, &paused, approve.clone()).await;
    let finished = one(&events, "tool.finished");
    assert_eq!(finished["callId"], call_id.as_str());
    assert_eq!(finished["status"], "ok");
    assert_eq!(text(&events), "Deleted.");
    assert_eq!(
        mcp.calls(),
        [("delete".to_owned(), json!({ "page": "intro" }))]
    );
    let saved = messages(&app, &token, &chat).await;
    assert_eq!(roles(&saved), ["user", "assistant", "tool", "assistant"]);
    assert_eq!(saved[1]["status"], "complete");
    assert_eq!(saved[1]["parts"][0]["approval"], "approved");
    assert_eq!(saved[2]["parts"][0]["status"], "ok");
    assert_eq!(content(&saved[3]), "Deleted.");
    let sent = completions(&provider).await;
    assert_eq!(sent.len(), 2);
    assert_eq!(
        sent[1]["tools"], sent[0]["tools"],
        "the paused turn's snapshot"
    );
    let provider_call = sent_call(&sent[1]);
    assert_eq!(
        sent_result(&sent[1], &provider_call),
        r#"delete {"page":"intro"}"#
    );

    // A decision applies once.
    let again = refused(&app, &token, &chat, &paused, approve).await;
    assert_eq!(again.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(again.body["code"], "NOT_AWAITING_APPROVAL");
    // "Once" approves this call only: the next call asks again.
    script(&provider, calls(&[("docs__delete", r#"{"page":"two"}"#)])).await;
    pause(&app, &token, &chat, "And page two").await;
    assert_eq!(mcp.calls().len(), 1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn a_denied_call_never_runs_and_the_model_reads_the_reason(pool: DbPool) {
    let (app, token, model, provider) = start(pool).await;
    let (mcp, server) = docs(&app, &token, "ask").await;
    let chat = tool_chat(&app, &token, &model, &server).await;
    script(&provider, calls(&[("docs__delete", r#"{"page":"intro"}"#)])).await;
    let (paused, call_id) = pause(&app, &token, &chat, "Delete the intro").await;

    script(&provider, answer("Okay, I left it.")).await;
    let deny = json!([{ "callId": call_id, "decision": "deny", "reason": "  Not now.  " }]);
    let events = decide(&app, &token, &chat, &paused, deny).await;
    let finished = one(&events, "tool.finished");
    assert_eq!(finished["status"], "denied");
    assert_eq!(
        finished["output"],
        json!([{ "type": "text", "text": "Not now." }])
    );
    assert_eq!(text(&events), "Okay, I left it.");
    assert!(mcp.calls().is_empty(), "a denied call never runs");

    let saved = messages(&app, &token, &chat).await;
    assert_eq!(roles(&saved), ["user", "assistant", "tool", "assistant"]);
    assert_eq!(saved[1]["parts"][0]["approval"], "denied");
    assert_eq!(saved[1]["parts"][0]["denyReason"], "Not now.");
    assert_eq!(saved[2]["parts"][0]["status"], "denied");
    let sent = completions(&provider).await;
    let provider_call = sent_call(&sent[1]);
    assert_eq!(sent_result(&sent[1], &provider_call), "Not now.");

    // Without a reason the model reads the default denial.
    script(&provider, calls(&[("docs__delete", r#"{"page":"two"}"#)])).await;
    let (paused, call_id) = pause(&app, &token, &chat, "Delete page two").await;
    script(&provider, answer("Fine.")).await;
    let deny = json!([{ "callId": call_id, "decision": "deny" }]);
    decide(&app, &token, &chat, &paused, deny).await;
    let sent = completions(&provider).await;
    let last = sent.last().expect("a request");
    assert_eq!(sent_result(last, &sent_call(last)), "Denied by the user.");
    assert!(mcp.calls().is_empty());
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn approving_for_the_chat_runs_later_calls_of_that_tool_without_asking(pool: DbPool) {
    let (app, token, model, provider) = start(pool).await;
    let (mcp, server) = docs(&app, &token, "ask").await;
    let chat = tool_chat(&app, &token, &model, &server).await;
    script(&provider, calls(&[("docs__delete", r#"{"page":"one"}"#)])).await;
    let (paused, call_id) = pause(&app, &token, &chat, "Delete page one").await;

    script(&provider, answer("Deleted.")).await;
    let approve = json!([{ "callId": call_id, "decision": "approve", "scope": "chat" }]);
    decide(&app, &token, &chat, &paused, approve).await;
    let details = call(
        &app,
        &token,
        Method::GET,
        &format!("/api/chats/{chat}"),
        None,
    )
    .await;
    assert_eq!(
        details["data"]["approvedTools"],
        json!([{ "serverId": server, "name": "delete" }])
    );

    script(&provider, calls(&[("docs__delete", r#"{"page":"two"}"#)])).await;
    script(&provider, answer("Deleted again.")).await;
    let events = say(&app, &token, &chat, "And page two").await;
    assert_eq!(one(&events, "tool.requested")["needsApproval"], false);
    assert_eq!(one(&events, "tool.finished")["status"], "ok");
    assert_eq!(text(&events), "Deleted again.");
    assert_eq!(
        mcp.calls(),
        [
            ("delete".to_owned(), json!({ "page": "one" })),
            ("delete".to_owned(), json!({ "page": "two" })),
        ]
    );

    // The approval belongs to this chat: another chat still asks.
    let fresh = tool_chat(&app, &token, &model, &server).await;
    script(&provider, calls(&[("docs__delete", r#"{"page":"three"}"#)])).await;
    pause(&app, &token, &fresh, "Delete page three").await;
    assert_eq!(mcp.calls().len(), 2);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn a_new_message_while_a_call_waits_skips_the_call(pool: DbPool) {
    let (app, token, model, provider) = start(pool).await;
    let (mcp, server) = docs(&app, &token, "ask").await;
    let chat = tool_chat(&app, &token, &model, &server).await;
    script(&provider, calls(&[("docs__delete", r#"{"page":"intro"}"#)])).await;
    let (paused, call_id) = pause(&app, &token, &chat, "Delete the intro").await;

    script(&provider, answer("Sure, something else then.")).await;
    let events = say(&app, &token, &chat, "Never mind").await;
    assert_eq!(text(&events), "Sure, something else then.");
    assert!(mcp.calls().is_empty(), "the skipped call never runs");

    let saved = messages(&app, &token, &chat).await;
    assert_eq!(
        roles(&saved),
        ["user", "assistant", "tool", "user", "assistant"]
    );
    assert_eq!(saved[1]["parts"][0]["approval"], "denied");
    assert_eq!(saved[1]["parts"][0]["denyReason"], "Skipped by user.");
    let result = &saved[2]["parts"][0];
    assert_eq!(result["callId"], call_id.as_str());
    assert_eq!(result["status"], "denied");
    assert_eq!(content(&saved[3]), "Never mind");
    let list = call(&app, &token, Method::GET, "/api/chats", None).await;
    assert_eq!(list["items"][0]["awaitingApproval"], false);

    // The model reads the skipped call's result before the new message.
    let sent = completions(&provider).await;
    let request = &sent[1];
    let provider_call = sent_call(request);
    assert_eq!(sent_result(request, &provider_call), "Skipped by user.");
    let last = request["messages"]
        .as_array()
        .expect("messages")
        .last()
        .expect("a message");
    assert_eq!(last["role"], "user");
    assert!(last.to_string().contains("Never mind"));

    // The skipped round is closed: a late decision changes nothing.
    let late = refused(
        &app,
        &token,
        &chat,
        &paused,
        json!([{ "callId": call_id, "decision": "approve" }]),
    )
    .await;
    assert_eq!(late.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(late.body["code"], "NOT_AWAITING_APPROVAL");
    assert!(mcp.calls().is_empty());
}

/// Opens `GET /api/events` for `token`.
async fn events(app: &Router, token: &str) -> Events {
    match open(
        app.clone(),
        request(Method::GET, "/api/events", Some(token), None),
    )
    .await
    {
        Ok(events) => events,
        Err(response) => panic!("events: {} {}", response.status, response.body),
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn the_event_stream_reports_runs_approvals_and_chat_changes_to_their_owner(pool: DbPool) {
    let (app, token, model, provider) = start(pool).await;
    let (_mcp, server) = docs(&app, &token, "ask").await;
    let other = other_user(&app, &token, "bob").await;
    let mut mine = events(&app, &token).await;
    let mut theirs = events(&app, &other).await;

    let chat = tool_chat(&app, &token, &model, &server).await;
    assert_eq!(
        mine.next().await,
        Some(json!({ "type": "chat.updated", "chatId": chat }))
    );
    script(&provider, calls(&[("docs__delete", r#"{"page":"intro"}"#)])).await;
    let (paused, call_id) = pause(&app, &token, &chat, "Delete the intro").await;
    assert_eq!(
        mine.next().await,
        Some(json!({ "type": "run.started", "chatId": chat }))
    );
    assert_eq!(
        mine.next().await,
        Some(json!({ "type": "approval.pending", "chatId": chat, "messageId": paused }))
    );
    assert_eq!(
        mine.next().await,
        Some(json!({ "type": "run.finished", "chatId": chat, "status": "awaiting_approval" }))
    );

    script(&provider, answer("Deleted.")).await;
    let approve = json!([{ "callId": call_id, "decision": "approve" }]);
    decide(&app, &token, &chat, &paused, approve).await;
    assert_eq!(
        mine.next().await,
        Some(json!({ "type": "run.started", "chatId": chat }))
    );
    assert_eq!(
        mine.next().await,
        Some(json!({ "type": "run.finished", "chatId": chat, "status": "complete" }))
    );

    // Reading the chat clears its unread mark on every device; a rename changes it too.
    call(
        &app,
        &token,
        Method::POST,
        &format!("/api/chats/{chat}/read"),
        None,
    )
    .await;
    let update = Some(json!({ "title": "Cleanup" }));
    call(
        &app,
        &token,
        Method::PATCH,
        &format!("/api/chats/{chat}"),
        update,
    )
    .await;
    for _ in 0..2 {
        assert_eq!(
            mine.next().await,
            Some(json!({ "type": "chat.updated", "chatId": chat }))
        );
    }

    // A stream that connects while a run is live hears of it first.
    next(&provider, answer("Slow."), Duration::from_millis(500)).await;
    let mut live = turn(
        &app,
        &token,
        &chat,
        "messages",
        Some(json!({ "text": "Again" })),
    )
    .await;
    let mut late = events(&app, &token).await;
    assert_eq!(
        late.next().await,
        Some(json!({ "type": "run.started", "chatId": chat }))
    );
    assert_eq!(text(&live.rest().await), "Slow.");
    // The run's own `run.started` may come too, when it started as the stream connected.
    let mut after = late.next().await;
    if after == Some(json!({ "type": "run.started", "chatId": chat })) {
        after = late.next().await;
    }
    assert_eq!(
        after,
        Some(json!({ "type": "run.finished", "chatId": chat, "status": "complete" }))
    );
    assert_eq!(
        mine.next().await,
        Some(json!({ "type": "run.started", "chatId": chat }))
    );

    // Another user heard none of it: their first event is their own.
    let created = call(&app, &other, Method::POST, "/api/chats", Some(json!({}))).await;
    let own = created["data"]["id"].as_str().expect("an id");
    assert_eq!(
        theirs.next().await,
        Some(json!({ "type": "chat.updated", "chatId": own }))
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn a_server_turned_off_is_never_listed_or_offered(pool: DbPool) {
    // A chat whose tool server is turned off sends no tools and never reaches the server.
    let (app, token, model, provider) = start(pool).await;
    let (mcp, server) = docs(&app, &token, "auto").await;
    let chat = tool_chat(&app, &token, &model, &server).await;
    let off = Some(json!({ "enabled": false }));
    call(
        &app,
        &token,
        Method::PATCH,
        &format!("/api/tool-servers/{server}"),
        off,
    )
    .await;
    script(&provider, answer("Plain.")).await;
    let before = mcp.requests();
    let events = say(&app, &token, &chat, "Hi").await;
    assert_eq!(text(&events), "Plain.");
    assert_eq!(
        mcp.requests(),
        before,
        "a server turned off is never listed"
    );
    let sent = completions(&provider).await;
    assert!(
        sent[0].get("tools").is_none_or(Value::is_null),
        "{}",
        sent[0]
    );
}
