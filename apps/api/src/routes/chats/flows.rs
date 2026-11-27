//! The chat flows of P3.1 end to end (`make test-db`): reattaching after a `seq`, a run that
//! outlives its client (also one whose client leaves while the turn is prepared), stop, one run
//! per chat and three per user, regenerate, retry, edit and resend, a restart's recovery, the
//! shutdown drain, and Continue after the tool round limit. rustauth and Copilot run on a
//! throwaway Postgres database; the provider is wiremock where whole answers do, and the
//! scripted [`LlmServer`] where a run must stay live while the test acts.

use std::time::{Duration, Instant};

use axum::Router;
use axum::http::{Method, StatusCode};
use copilot_core::INTERRUPTED;
use copilot_database::DbPool;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::App;
use crate::helpers::bootstrap::recover;
use crate::helpers::llm_server::LlmServer;
use crate::helpers::runs::SERVER_STOPPING;
use crate::helpers::testing::{
    self, Events, config, live, model_at, open, other_user, register, request, send,
};

/// The app on `pool` with the first account (the administrator) and a model on `base_url`:
/// the app, the account's token, and the model's id.
pub(crate) async fn setup(pool: DbPool, base_url: &str) -> (App, String, String) {
    let app = live(pool, config()).await;
    let token = register(&app.router, "admin").await;
    let model = model_at(&app.router, &token, base_url).await;
    (app, token, model)
}

/// A JSON request that must succeed; its body.
pub(crate) async fn call(
    app: &Router,
    token: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> Value {
    let response = send(app.clone(), request(method.clone(), uri, Some(token), body)).await;
    assert!(
        response.status.is_success(),
        "{method} {uri}: {} {}",
        response.status,
        response.body
    );
    response.body
}

pub(crate) async fn new_chat(app: &Router, token: &str, model: &str) -> String {
    let chat = call(
        app,
        token,
        Method::POST,
        "/api/chats",
        Some(json!({ "modelId": model })),
    )
    .await;
    chat["data"]["id"].as_str().expect("an id").to_owned()
}

/// Opens a turn route (`messages`, `regenerate`, `retry`) of `chat` as an event stream.
pub(crate) async fn turn(
    app: &Router,
    token: &str,
    chat: &str,
    route: &str,
    body: Option<Value>,
) -> Events {
    match open(
        app.clone(),
        request(
            Method::POST,
            &format!("/api/chats/{chat}/{route}"),
            Some(token),
            body,
        ),
    )
    .await
    {
        Ok(events) => events,
        Err(response) => panic!("{route}: {} {}", response.status, response.body),
    }
}

/// Sends `text` and reads the turn to its end; the events.
pub(crate) async fn say(app: &Router, token: &str, chat: &str, text: &str) -> Vec<Value> {
    let mut events = turn(app, token, chat, "messages", Some(json!({ "text": text }))).await;
    let all = events.rest().await;
    assert_eq!(all.last().map(|event| &event["type"]), Some(&json!("done")));
    all
}

/// The text of the text deltas in `events`.
pub(crate) fn text(events: &[Value]) -> String {
    events
        .iter()
        .filter(|event| event["type"] == "text.delta")
        .map(|event| event["text"].as_str().expect("text"))
        .collect()
}

pub(crate) fn seqs(events: &[Value]) -> Vec<u64> {
    events
        .iter()
        .map(|event| event["seq"].as_u64().expect("a seq"))
        .collect()
}

/// The chat's messages once no run is live.
pub(crate) async fn settled(app: &Router, token: &str, chat: &str) -> Vec<Value> {
    let started = Instant::now();
    loop {
        let stream = send(
            app.clone(),
            request(
                Method::GET,
                &format!("/api/chats/{chat}/stream"),
                Some(token),
                None,
            ),
        )
        .await;
        if stream.status == StatusCode::NO_CONTENT {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "the run ends");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    messages(app, token, chat).await
}

pub(crate) async fn messages(app: &Router, token: &str, chat: &str) -> Vec<Value> {
    let list = call(
        app,
        token,
        Method::GET,
        &format!("/api/chats/{chat}/messages"),
        None,
    )
    .await;
    list["items"].as_array().expect("items").clone()
}

/// A message's text parts, joined.
pub(crate) fn content(message: &Value) -> String {
    message["parts"]
        .as_array()
        .expect("parts")
        .iter()
        .filter(|part| part["type"] == "text")
        .map(|part| part["text"].as_str().expect("text"))
        .collect()
}

/// Waits until the provider has received `count` completion requests.
pub(crate) async fn requested(llm: &LlmServer, count: usize) {
    let started = Instant::now();
    while llm.requests().len() < count {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "request {count} arrives"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// `GET /chats/{chat}/stream`, with `after` when given.
pub(crate) async fn attach(
    app: &Router,
    token: &str,
    chat: &str,
    after: Option<u64>,
) -> Option<Events> {
    let query = after.map_or_else(String::new, |seq| format!("?after={seq}"));
    match open(
        app.clone(),
        request(
            Method::GET,
            &format!("/api/chats/{chat}/stream{query}"),
            Some(token),
            None,
        ),
    )
    .await
    {
        Ok(events) => Some(events),
        Err(response) if response.status == StatusCode::NO_CONTENT => None,
        Err(response) => panic!("stream: {} {}", response.status, response.body),
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn a_reattached_stream_resumes_after_its_last_seq_without_repeating_text(pool: DbPool) {
    let llm = LlmServer::start().await;
    let (app, token, model) = setup(pool, &llm.url).await;
    let app = app.router;
    let chat = new_chat(&app, &token, &model).await;
    let writer = llm.live();
    let mut first = turn(
        &app,
        &token,
        &chat,
        "messages",
        Some(json!({ "text": "Hi" })),
    )
    .await;
    writer.text("Hel");
    writer.text("lo");
    let seen = first.until("text.delta").await;
    let last = *seqs(&seen).last().expect("events");
    assert_eq!(text(&seen), "Hel");
    // The connection drops; the run goes on and buffers what follows.
    drop(first);

    let bob = other_user(&app, &token, "bob").await;
    let theirs = send(
        app.clone(),
        request(
            Method::GET,
            &format!("/api/chats/{chat}/stream"),
            Some(&bob),
            None,
        ),
    )
    .await;
    assert_eq!(
        theirs.status,
        StatusCode::NOT_FOUND,
        "never another user's run"
    );

    let mut resumed = attach(&app, &token, &chat, Some(last)).await.expect("live");
    let mut whole = attach(&app, &token, &chat, None).await.expect("live");
    writer.text(", world!");
    writer.finish("stop");

    let rest = resumed.rest().await;
    assert_eq!(text(&rest), "lo, world!", "exactly the rest: {rest:?}");
    assert!(seqs(&rest).iter().all(|seq| *seq > last));
    assert!(seqs(&rest).windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        rest.last().map(|event| &event["type"]),
        Some(&json!("done"))
    );
    let replayed = whole.rest().await;
    assert_eq!(replayed[0]["type"], "message.started", "from the start");
    assert_eq!(replayed[0]["seq"], 1);
    assert_eq!(text(&replayed), "Hello, world!");

    let saved = settled(&app, &token, &chat).await;
    assert_eq!(saved[1]["status"], "complete");
    assert_eq!(content(&saved[1]), "Hello, world!");
    assert!(
        attach(&app, &token, &chat, Some(last)).await.is_none(),
        "204 once the run is over"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn a_run_goes_on_when_its_client_disconnects(pool: DbPool) {
    let llm = LlmServer::start().await;
    let (app, token, model) = setup(pool, &llm.url).await;
    let app = app.router;
    let chat = new_chat(&app, &token, &model).await;
    let writer = llm.live();
    let mut events = turn(
        &app,
        &token,
        &chat,
        "messages",
        Some(json!({ "text": "Hi" })),
    )
    .await;
    events.until("message.started").await;
    drop(events);
    requested(&llm, 1).await;

    writer.text("Still ");
    writer.text("here");
    writer.finish("stop");
    let saved = settled(&app, &token, &chat).await;
    assert_eq!(saved.len(), 2);
    assert_eq!(saved[1]["status"], "complete");
    assert_eq!(content(&saved[1]), "Still here");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn a_turn_starts_when_its_client_leaves_while_it_is_prepared(pool: DbPool) {
    let server: MockServer = testing::provider().await;
    mock_answer(&server, "Mocked").await;
    let (app, token, model) = setup(pool.clone(), &format!("{}/v1", server.uri())).await;
    let app = app.router;
    let chat = new_chat(&app, &token, &model).await;
    let id: uuid::Uuid = chat.parse().expect("a uuid");

    // Hold the chat row, so `prepare` waits on its first write.
    let mut held = pool.begin().await.expect("a transaction");
    sqlx::query("SELECT id FROM chats WHERE id = $1 FOR UPDATE")
        .bind(id)
        .execute(&mut *held)
        .await
        .expect("the chat row");
    let sending = send(
        app.clone(),
        request(
            Method::POST,
            &format!("/api/chats/{chat}/messages"),
            Some(&token),
            Some(json!({ "text": "Hi" })),
        ),
    );
    // The client gives up while the turn is prepared.
    assert!(
        tokio::time::timeout(Duration::from_millis(300), sending)
            .await
            .is_err(),
        "prepare waits for the row"
    );
    assert!(
        attach(&app, &token, &chat, None).await.is_none(),
        "a turn being prepared is idle to streams"
    );
    held.rollback().await.expect("the row is free");

    let started = Instant::now();
    let saved = loop {
        let saved = messages(&app, &token, &chat).await;
        if saved.len() == 2 && saved[1]["status"] == "complete" {
            break saved;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the turn runs without its client: {saved:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(content(&saved[0]), "Hi");
    assert_eq!(content(&saved[1]), "Mocked");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn stop_saves_the_partial_answer_and_frees_the_chat(pool: DbPool) {
    let llm = LlmServer::start().await;
    let (app, token, model) = setup(pool, &llm.url).await;
    let app = app.router;
    let chat = new_chat(&app, &token, &model).await;
    let writer = llm.live();
    let mut events = turn(
        &app,
        &token,
        &chat,
        "messages",
        Some(json!({ "text": "Hi" })),
    )
    .await;
    writer.text("Partial");
    events.until("text.delta").await;

    let bob = other_user(&app, &token, "bob").await;
    let stop = format!("/api/chats/{chat}/stop");
    let theirs = send(app.clone(), request(Method::POST, &stop, Some(&bob), None)).await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND);
    assert!(
        attach(&app, &token, &chat, None).await.is_some(),
        "still live"
    );

    let stopped = send(
        app.clone(),
        request(Method::POST, &stop, Some(&token), None),
    )
    .await;
    assert_eq!(stopped.status, StatusCode::NO_CONTENT, "{}", stopped.body);
    // The answer is saved by the time stop answers.
    let saved = messages(&app, &token, &chat).await;
    assert_eq!(saved[1]["status"], "cancelled");
    assert_eq!(content(&saved[1]), "Partial");
    let rest = events.rest().await;
    let finished = rest
        .iter()
        .find(|event| event["type"] == "message.finished")
        .expect("message.finished");
    assert_eq!(finished["status"], "cancelled");
    assert_eq!(
        rest.last().map(|event| &event["type"]),
        Some(&json!("done"))
    );
    writer.text("too late");

    let idle = send(
        app.clone(),
        request(Method::POST, &stop, Some(&token), None),
    )
    .await;
    assert_eq!(idle.status, StatusCode::NO_CONTENT, "stopping an idle chat");
    llm.reply(&["Again"]);
    let again = say(&app, &token, &chat, "Once more").await;
    assert_eq!(text(&again), "Again", "the chat is free");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn a_chat_answers_once_and_a_user_runs_at_most_three_chats(pool: DbPool) {
    let llm = LlmServer::start().await;
    let (app, token, model) = setup(pool, &llm.url).await;
    let app = app.router;
    let mut live = Vec::new();
    let mut chats = Vec::new();
    for _ in 0..3 {
        let chat = new_chat(&app, &token, &model).await;
        let writer = llm.live();
        let events = turn(
            &app,
            &token,
            &chat,
            "messages",
            Some(json!({ "text": "Hi" })),
        )
        .await;
        live.push((writer, events));
        chats.push(chat);
    }
    for route in ["messages", "regenerate", "retry"] {
        let busy = send(
            app.clone(),
            request(
                Method::POST,
                &format!("/api/chats/{}/{route}", chats[0]),
                Some(&token),
                Some(json!({ "text": "Again" })),
            ),
        )
        .await;
        assert_eq!(busy.status, StatusCode::UNPROCESSABLE_ENTITY, "{route}");
        assert_eq!(busy.body["code"], "CHAT_BUSY", "{route}");
    }
    let fourth = new_chat(&app, &token, &model).await;
    let refused = send(
        app.clone(),
        request(
            Method::POST,
            &format!("/api/chats/{fourth}/messages"),
            Some(&token),
            Some(json!({ "text": "One more" })),
        ),
    )
    .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused.body["code"], "TOO_MANY_RUNS");
    assert!(
        messages(&app, &token, &fourth).await.is_empty(),
        "a refused turn writes nothing"
    );

    // Another user has their own limit.
    let bob = other_user(&app, &token, "bob").await;
    let theirs = call(&app, &bob, Method::POST, "/api/chats", Some(json!({}))).await;
    let theirs = theirs["data"]["id"].as_str().expect("an id").to_owned();
    let no_model = send(
        app.clone(),
        request(
            Method::POST,
            &format!("/api/chats/{theirs}/messages"),
            Some(&bob),
            Some(json!({ "text": "Hi" })),
        ),
    )
    .await;
    assert_eq!(
        no_model.status,
        StatusCode::BAD_REQUEST,
        "reserved and prepared, never TOO_MANY_RUNS: {}",
        no_model.body
    );

    let (writer, mut events) = live.remove(0);
    writer.text("Done");
    writer.finish("stop");
    events.rest().await;
    llm.reply(&["Now"]);
    let now = say(&app, &token, &fourth, "One more").await;
    assert_eq!(text(&now), "Now", "a slot is free again");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn regenerate_replaces_every_message_after_the_last_question(pool: DbPool) {
    let llm = LlmServer::start().await;
    let (app, token, model) = setup(pool, &llm.url).await;
    let app = app.router;
    let chat = new_chat(&app, &token, &model).await;
    llm.reply(&["First"]);
    say(&app, &token, &chat, "Question").await;

    let bob = other_user(&app, &token, "bob").await;
    let theirs = send(
        app.clone(),
        request(
            Method::POST,
            &format!("/api/chats/{chat}/regenerate"),
            Some(&bob),
            None,
        ),
    )
    .await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND);

    llm.reply(&["Second"]);
    let mut events = turn(&app, &token, &chat, "regenerate", None).await;
    let all = events.rest().await;
    assert_eq!(text(&all), "Second");
    let saved = messages(&app, &token, &chat).await;
    assert_eq!(saved.len(), 2, "{saved:?}");
    assert_eq!(content(&saved[0]), "Question");
    assert_eq!(content(&saved[1]), "Second");

    let sent = &llm.requests()[1];
    let roles: Vec<&str> = sent["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .map(|message| message["role"].as_str().expect("a role"))
        .collect();
    assert_eq!(roles, ["system", "user"], "the old answer is not sent");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn retry_answers_a_failed_answer_again(pool: DbPool) {
    let llm = LlmServer::start().await;
    let (app, token, model) = setup(pool, &llm.url).await;
    let app = app.router;
    let chat = new_chat(&app, &token, &model).await;
    let retry = format!("/api/chats/{chat}/retry");
    llm.reply(&["Fine"]);
    say(&app, &token, &chat, "Hello").await;
    let nothing = send(
        app.clone(),
        request(Method::POST, &retry, Some(&token), None),
    )
    .await;
    assert_eq!(nothing.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(nothing.body["code"], "NOTHING_TO_RETRY");

    // The provider ends the answer before it finishes.
    let writer = llm.live();
    writer.text("Half");
    drop(writer);
    let failed = say(&app, &token, &chat, "Question").await;
    assert!(
        failed.iter().any(|event| event["type"] == "error"),
        "{failed:?}"
    );
    let saved = messages(&app, &token, &chat).await;
    assert_eq!(saved[3]["status"], "error");
    assert_eq!(content(&saved[3]), "Half", "what streamed is kept");

    llm.reply(&["Recovered"]);
    let mut events = turn(&app, &token, &chat, "retry", None).await;
    assert_eq!(text(&events.rest().await), "Recovered");
    let saved = messages(&app, &token, &chat).await;
    assert_eq!(saved.len(), 4, "the failed answer is replaced: {saved:?}");
    assert_eq!(content(&saved[2]), "Question");
    assert_eq!(saved[3]["status"], "complete");
    assert_eq!(content(&saved[3]), "Recovered");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn an_edit_resends_from_the_edited_message(pool: DbPool) {
    let llm = LlmServer::start().await;
    let (app, token, model) = setup(pool, &llm.url).await;
    let app = app.router;
    let chat = new_chat(&app, &token, &model).await;
    llm.reply(&["A"]);
    say(&app, &token, &chat, "First").await;
    llm.reply(&["B"]);
    say(&app, &token, &chat, "Second").await;
    let first = messages(&app, &token, &chat).await[0]["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    llm.reply(&["C"]);
    let mut events = turn(
        &app,
        &token,
        &chat,
        "messages",
        Some(json!({ "text": "Edited", "editFrom": first })),
    )
    .await;
    assert_eq!(text(&events.rest().await), "C");
    let saved = messages(&app, &token, &chat).await;
    let texts: Vec<String> = saved.iter().map(content).collect();
    assert_eq!(texts, ["Edited", "C"]);
    let sent = llm.requests()[2]["messages"].to_string();
    assert!(sent.contains("Edited"));
    assert!(
        !sent.contains("First") && !sent.contains("Second"),
        "the replaced messages are gone: {sent}"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn a_restart_marks_live_answers_interrupted_and_retry_answers_again(pool: DbPool) {
    let llm = LlmServer::start().await;
    let (before, token, model) = setup(pool.clone(), &llm.url).await;
    let chat = new_chat(&before.router, &token, &model).await;
    // The old process is answering when it stops: its answer stays `streaming` in the database.
    let writer = llm.live();
    let mut events = turn(
        &before.router,
        &token,
        &chat,
        "messages",
        Some(json!({ "text": "Hi" })),
    )
    .await;
    writer.text("Cut");
    events.until("text.delta").await;
    let saved = messages(&before.router, &token, &chat).await;
    assert_eq!(saved[1]["status"], "streaming");

    // The new process recovers before it serves.
    let after = live(pool, config()).await;
    assert_eq!(recover(&after.state).await.expect("recovered"), 1);
    let app = after.router;
    let saved = messages(&app, &token, &chat).await;
    assert_eq!(saved[1]["status"], "error");
    assert_eq!(saved[1]["error"], INTERRUPTED);
    assert!(
        attach(&app, &token, &chat, None).await.is_none(),
        "idle here"
    );
    assert_eq!(recover(&after.state).await.expect("recovered"), 0, "once");

    llm.reply(&["Answered"]);
    let mut retried = turn(&app, &token, &chat, "retry", None).await;
    assert_eq!(text(&retried.rest().await), "Answered");
    let saved = messages(&app, &token, &chat).await;
    assert_eq!(saved.len(), 2);
    assert_eq!(saved[1]["status"], "complete");
    drop(writer);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn shutdown_lets_runs_finish_then_cancels_the_rest(pool: DbPool) {
    let llm = LlmServer::start().await;
    let (App { router: app, state }, token, model) = setup(pool, &llm.url).await;
    let quick = new_chat(&app, &token, &model).await;
    let slow = new_chat(&app, &token, &model).await;
    let quick_writer = llm.live();
    let mut quick_events = turn(
        &app,
        &token,
        &quick,
        "messages",
        Some(json!({ "text": "A" })),
    )
    .await;
    requested(&llm, 1).await;
    let slow_writer = llm.live();
    let mut slow_events = turn(
        &app,
        &token,
        &slow,
        "messages",
        Some(json!({ "text": "B" })),
    )
    .await;
    requested(&llm, 2).await;
    slow_writer.text("Half");
    slow_events.until("text.delta").await;

    let drained = state
        .runs
        .drain(Duration::from_secs(1), Duration::from_secs(10));
    let refused = send(
        app.clone(),
        request(
            Method::POST,
            &format!("/api/chats/{quick}/messages"),
            Some(&token),
            Some(json!({ "text": "C" })),
        ),
    )
    .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused.body["code"], SERVER_STOPPING);
    quick_writer.text("In time");
    quick_writer.finish("stop");
    // The client reads while the server drains: a run that finishes in the grace time ends
    // normally.
    let quick_rest = quick_events.rest().await;
    assert_eq!(text(&quick_rest), "In time");
    assert_eq!(
        quick_rest.last().map(|event| &event["type"]),
        Some(&json!("done"))
    );
    assert!(drained.await, "every run ended");
    assert!(state.shutdown.is_cancelled());
    // The slow run's stream ended with the shutdown token.
    slow_events.rest().await;
    let saved = messages(&app, &token, &quick).await;
    assert_eq!(saved[1]["status"], "complete");
    let saved = messages(&app, &token, &slow).await;
    assert_eq!(saved[1]["status"], "cancelled");
    assert_eq!(content(&saved[1]), "Half");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn continue_after_the_tool_round_limit_starts_a_new_turn(pool: DbPool) {
    let llm = LlmServer::start().await;
    let (app, token, model) = setup(pool, &llm.url).await;
    let app = app.router;
    let chat = new_chat(&app, &token, &model).await;
    // The model asks for a tool every round; it is not in the chat's snapshot, so each call
    // gets an error result and the turn goes on, until the eighth round.
    for _ in 0..8 {
        let writer = llm.live();
        writer.call(0, "gone__tool", "{}");
        writer.finish("tool_calls");
    }
    let events = say(&app, &token, &chat, "Loop").await;
    let finishes: Vec<&Value> = events
        .iter()
        .filter(|event| event["type"] == "message.finished")
        .map(|event| &event["finishReason"])
        .collect();
    assert_eq!(
        finishes.last(),
        Some(&&json!(null)),
        "the last tool message"
    );
    let saved = messages(&app, &token, &chat).await;
    assert_eq!(
        saved.len(),
        17,
        "the question, then eight answers with their results"
    );
    assert_eq!(saved[15]["finishReason"], "tool_limit");
    let limited = saved[16]["parts"][0].clone();
    assert_eq!(limited["type"], "toolResult");
    assert_eq!(limited["status"], "cancelled");

    // Continue is a new user message: the round count starts over.
    llm.reply(&["Done now"]);
    let more = say(&app, &token, &chat, "Continue.").await;
    assert_eq!(text(&more), "Done now");
    let saved = messages(&app, &token, &chat).await;
    assert_eq!(saved.len(), 19);
    assert_eq!(saved[18]["finishReason"], "stop");
    let sent = &llm.requests()[8]["messages"];
    let last = sent
        .as_array()
        .expect("messages")
        .last()
        .expect("a message");
    assert_eq!(last["role"], "user");
    assert!(last.to_string().contains("Continue."));
    assert_eq!(llm.requests().len(), 9);
}

/// A provider that answers every completion at once: the turn routes need no live provider.
#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn turn_routes_answer_through_wiremock(pool: DbPool) {
    let server: MockServer = testing::provider().await;
    mock_answer(&server, "Mocked").await;
    let (app, token, model) = setup(pool, &format!("{}/v1", server.uri())).await;
    let app = app.router;
    let chat = new_chat(&app, &token, &model).await;
    let first = say(&app, &token, &chat, "Hi").await;
    assert_eq!(text(&first), "Mocked");
    let mut regenerated = turn(&app, &token, &chat, "regenerate", None).await;
    assert_eq!(text(&regenerated.rest().await), "Mocked");
    let saved = messages(&app, &token, &chat).await;
    assert_eq!(saved.len(), 2);
    let received = server.received_requests().await.unwrap_or_default();
    let completions = received
        .iter()
        .filter(|request| request.url.path() == "/v1/chat/completions")
        .count();
    assert_eq!(completions, 2);
}

/// Answers every completion request on `server` with `text`.
async fn mock_answer(server: &MockServer, text: &str) {
    let body = [
        json!({ "choices": [{ "index": 0, "delta": { "content": text }, "finish_reason": null }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] }),
    ]
    .iter()
    .map(|chunk| format!("data: {chunk}\n\n"))
    .collect::<String>()
        + "data: [DONE]\n\n";
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(server)
        .await;
}
