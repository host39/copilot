//! The scheduler end to end (`make test-db`): rustauth and Copilot on a throwaway Postgres
//! database, wiremock (or the scripted [`LlmServer`]) as the provider, and ticks at chosen
//! instants instead of the clock.

use std::time::{Duration, Instant};

use axum::Router;
use axum::http::{Method, StatusCode};
use chrono::{DateTime, Utc};
use copilot_core::PrivateNetwork;
use copilot_database::DbPool;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::config::AppState;
use crate::helpers::auth::testing::identity;
use crate::helpers::bootstrap::Adapters;
use crate::helpers::llm_server::LlmServer;
use crate::helpers::scheduler::Scheduler;
use crate::helpers::testing::{self, config, live, other_user, request, send};
use crate::routes::chats::flows::{
    attach, call, content, messages, new_chat, requested, say, setup, text,
};
use crate::routes::chats::tools::{answer, calls, completions, events, next, script};
use crate::{App, AppDependencies, assemble};

/// The app with a wiremock provider: the app, the first account's token, the model's id, and
/// the provider to script.
async fn start(pool: DbPool) -> (App, String, String, MockServer) {
    let provider = testing::provider().await;
    let (app, token, model) = setup(pool, &format!("{}/v1", provider.uri())).await;
    (app, token, model, provider)
}

/// A schedule of `token`'s user every day at 09:00 UTC on `model`, with `extra` fields; its JSON.
async fn schedule(app: &Router, token: &str, model: &str, extra: Value) -> Value {
    let mut body = json!({
        "name": "Digest",
        "cron": "0 9 * * *",
        "timezone": "UTC",
        "prompt": "Summarize the news.",
        "modelId": model,
    });
    for (key, value) in extra.as_object().expect("fields") {
        body[key] = value.clone();
    }
    call(app, token, Method::POST, "/api/schedules", Some(body)).await["data"].clone()
}

async fn read(app: &Router, token: &str, schedule: &Value) -> Value {
    let id = schedule["id"].as_str().expect("an id");
    call(
        app,
        token,
        Method::GET,
        &format!("/api/schedules/{id}"),
        None,
    )
    .await["data"]
        .clone()
}

/// When `schedule` runs next.
fn due(schedule: &Value) -> DateTime<Utc> {
    schedule["nextRunAt"]
        .as_str()
        .expect("a next run")
        .parse()
        .expect("an RFC 3339 time")
}

fn instant(value: &Value) -> Option<DateTime<Utc>> {
    value
        .as_str()
        .map(|text| text.parse().expect("an RFC 3339 time"))
}

/// The chats of `schedule`'s runs.
async fn runs(app: &Router, token: &str, schedule: &Value) -> Vec<Value> {
    let id = schedule["id"].as_str().expect("an id");
    let page = call(
        app,
        token,
        Method::GET,
        &format!("/api/chats?scheduleId={id}"),
        None,
    )
    .await;
    page["items"].as_array().expect("items").clone()
}

/// Waits until every scheduled run ended and recorded how.
async fn quiet(state: &AppState) {
    let started = Instant::now();
    while !state.tasks.is_empty() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the runs end and record"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn second() -> chrono::Duration {
    chrono::Duration::seconds(1)
}

#[tokio::test]
async fn a_run_past_the_time_limit_is_stopped_and_fails() {
    use copilot_core::shared::constants::SCHEDULED_RUN_MAX_SECS;
    use copilot_core::{MessageStatus, RUN_TIMED_OUT, RunOutcome, TurnOutcome};

    use crate::helpers::scheduler::ended;

    assert!(
        RUN_TIMED_OUT.contains(&format!("{} minutes", SCHEDULED_RUN_MAX_SECS / 60)),
        "the message names the limit"
    );
    let outcome = |status| TurnOutcome {
        message_id: None,
        status,
        finish_reason: None,
        error: None,
    };
    // A run that stops when it is cancelled, saving its answer as cancelled.
    let cancel = CancellationToken::new();
    let slow = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            cancel.cancelled().await;
            Some(outcome(MessageStatus::Cancelled))
        }
    });
    let limit = Duration::from_millis(50);
    assert_eq!(
        ended(slow, &cancel, limit, || false).await,
        RunOutcome::Failed(RUN_TIMED_OUT.to_owned())
    );
    assert!(cancel.is_cancelled());

    let quick = tokio::spawn(async move { Some(outcome(MessageStatus::Complete)) });
    let untouched = CancellationToken::new();
    assert_eq!(
        ended(quick, &untouched, limit, || false).await,
        RunOutcome::Ok
    );
    assert!(
        !untouched.is_cancelled(),
        "a run within the limit is left alone"
    );
    let failed = tokio::spawn(async { None });
    assert!(matches!(
        ended(failed, &untouched, limit, || false).await,
        RunOutcome::Failed(_)
    ));

    // Cancelled by a shutdown: not the schedule's failure. A user's Stop still is.
    let stopped = || tokio::spawn(async move { Some(outcome(MessageStatus::Cancelled)) });
    assert_eq!(
        ended(stopped(), &untouched, limit, || true).await,
        RunOutcome::Ended(copilot_core::RUN_INTERRUPTED.to_owned())
    );
    assert!(matches!(
        ended(stopped(), &untouched, limit, || false).await,
        RunOutcome::Failed(_)
    ));
}

#[tokio::test]
async fn the_loop_stops_on_shutdown() {
    let shutdown = CancellationToken::new();
    let config = config();
    let db = copilot_database::create_pool(&config.db);
    let App { state, .. } = assemble(AppDependencies {
        config,
        db,
        identity: identity().await,
        adapters: Adapters::unavailable(),
        shutdown: shutdown.clone(),
    });
    // The database is unreachable: the first tick fails, is logged, and the loop goes on.
    let running = Scheduler::new(state).start(shutdown.clone());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !running.is_finished(),
        "a failed tick does not end the loop"
    );
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), running)
        .await
        .expect("the loop stops")
        .expect("without a panic");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn a_due_schedule_runs_as_its_owner_in_a_new_chat_with_a_streamed_answer(pool: DbPool) {
    let (app, token, model, provider) = start(pool).await;
    let state = app.state.clone();
    let app = app.router;
    let schedule = schedule(&app, &token, &model, json!({})).await;
    let id = schedule["id"].as_str().expect("an id").to_owned();
    let due = due(&schedule);
    let mut heard = events(&app, &token).await;
    let scheduler = Scheduler::new(state.clone());

    assert!(
        scheduler.tick(due - second()).await.is_empty(),
        "the clock decides: not due yet"
    );
    // The answer takes a moment, so the run is still live when the client attaches.
    next(
        &provider,
        answer("Three things happened."),
        Duration::from_millis(300),
    )
    .await;
    let now = due + second();
    let started = scheduler.tick(now).await;
    assert_eq!(started.len(), 1);
    let chat = started[0].to_string();
    assert!(scheduler.tick(now).await.is_empty(), "claimed once");

    let mut stream = attach(&app, &token, &chat, None)
        .await
        .expect("the run is live");
    let streamed = stream.rest().await;
    assert_eq!(text(&streamed), "Three things happened.");
    assert_eq!(
        streamed.last().map(|event| &event["type"]),
        Some(&json!("done"))
    );
    quiet(&state).await;

    let saved = messages(&app, &token, &chat).await;
    assert_eq!(saved.len(), 2);
    assert_eq!(saved[0]["role"], "user");
    assert_eq!(content(&saved[0]), "Summarize the news.");
    assert_eq!(content(&saved[1]), "Three things happened.");
    let opened = call(
        &app,
        &token,
        Method::GET,
        &format!("/api/chats/{chat}"),
        None,
    )
    .await;
    assert_eq!(opened["data"]["scheduleId"], id.as_str());
    assert_eq!(
        opened["data"]["title"],
        format!("Digest · {}", now.date_naive())
    );
    assert_eq!(opened["data"]["modelId"], model.as_str());
    let listed = runs(&app, &token, &schedule).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["unread"], true, "nobody has read the run yet");

    let stored = read(&app, &token, &schedule).await;
    assert_eq!(stored["lastStatus"], "ok");
    assert_eq!(stored["consecutiveFailures"], 0);
    assert_eq!(instant(&stored["lastRunAt"]), Some(now));
    assert_eq!(
        instant(&stored["nextRunAt"]),
        Some(due + chrono::Duration::days(1)),
        "the next fire after the claim"
    );

    let request = &completions(&provider).await[0];
    assert!(
        request.get("tools").is_none(),
        "a standalone schedule has no tools"
    );

    let schedule_updated = json!({ "type": "schedule.updated", "scheduleId": id });
    assert_eq!(
        heard.next().await,
        Some(schedule_updated.clone()),
        "the claim"
    );
    assert_eq!(
        heard.next().await,
        Some(json!({ "type": "run.started", "chatId": chat }))
    );
    assert_eq!(
        heard.next().await,
        Some(json!({ "type": "run.finished", "chatId": chat, "status": "complete" }))
    );
    assert_eq!(heard.next().await, Some(schedule_updated), "the record");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn two_schedulers_on_one_database_fire_each_due_schedule_once(pool: DbPool) {
    let provider = testing::provider().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(answer("Done.")),
        )
        .mount(&provider)
        .await;
    let (first, token, model) = setup(pool.clone(), &format!("{}/v1", provider.uri())).await;
    // Another server process on the same database: its own registry and slots.
    let second_server = live(pool, config()).await;
    let schedules = [
        schedule(&first.router, &token, &model, json!({})).await,
        schedule(&first.router, &token, &model, json!({ "name": "Other" })).await,
    ];
    let due = due(&schedules[0]);

    for round in 0..3 {
        let now = due + chrono::Duration::days(round) + second();
        let (left, right) = (
            Scheduler::new(first.state.clone()),
            Scheduler::new(second_server.state.clone()),
        );
        let (left, right) = tokio::join!(
            tokio::spawn(async move { left.tick(now).await }),
            tokio::spawn(async move { right.tick(now).await }),
        );
        let started = left.expect("a tick").len() + right.expect("a tick").len();
        assert_eq!(started, 2, "round {round}: each due schedule once");
        quiet(&first.state).await;
        quiet(&second_server.state).await;
    }
    for schedule in &schedules {
        assert_eq!(runs(&first.router, &token, schedule).await.len(), 3);
    }
    assert_eq!(completions(&provider).await.len(), 6, "one answer per fire");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn at_most_two_scheduled_runs_execute_at_once_and_a_waiting_fire_runs_later(pool: DbPool) {
    let llm = LlmServer::start().await;
    let (app, token, model) = setup(pool, &llm.url).await;
    let state = app.state.clone();
    let app = app.router;
    let mut schedules = Vec::new();
    for name in ["One", "Two", "Three"] {
        schedules.push(schedule(&app, &token, &model, json!({ "name": name })).await);
    }
    // The provider answers in the order the requests come: the two runs, then the chat below.
    let writers = [llm.live(), llm.live()];
    let scheduler = Scheduler::new(state.clone());
    let now = due(&schedules[0]) + second();

    assert_eq!(scheduler.tick(now).await.len(), 2, "two slots");
    requested(&llm, 2).await;
    let later = now + chrono::Duration::seconds(30);
    assert!(
        scheduler.tick(later).await.is_empty(),
        "both slots are taken"
    );
    let third = schedules[2]["id"].as_str().expect("an id");
    let busy = send(
        app.clone(),
        request(
            Method::POST,
            &format!("/api/schedules/{third}/run"),
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(busy.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(busy.body["code"], "SCHEDULER_BUSY");
    // The owner's own chats do not wait for their schedules.
    let chat = new_chat(&app, &token, &model).await;
    let writer = llm.live();
    let chatting = tokio::spawn({
        let app = app.clone();
        let token = token.clone();
        async move { say(&app, &token, &chat, "Hello").await }
    });
    requested(&llm, 3).await;

    for writer in writers {
        writer.text("Done.");
        writer.finish("stop");
    }
    writer.text("Hi.");
    writer.finish("stop");
    assert_eq!(text(&chatting.await.expect("the chat")), "Hi.");
    quiet(&state).await;

    let waiting = llm.live();
    let started = scheduler.tick(later + chrono::Duration::seconds(30)).await;
    assert_eq!(started.len(), 1, "the fire that waited runs once, late");
    waiting.text("Done.");
    waiting.finish("stop");
    quiet(&state).await;
    for schedule in &schedules {
        let stored = read(&app, &token, schedule).await;
        assert_eq!(stored["lastStatus"], "ok", "{}", stored["name"]);
        assert_eq!(runs(&app, &token, schedule).await.len(), 1);
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn unattended_runs_never_see_the_schedule_tools(pool: DbPool) {
    let (app, token, model, provider) = start(pool).await;
    let state = app.state.clone();
    let app = app.router;
    let project = call(
        &app,
        &token,
        Method::POST,
        "/api/projects",
        Some(json!({ "name": "News", "builtinTools": ["schedules"] })),
    )
    .await;
    let project = project["data"]["id"].as_str().expect("an id").to_owned();
    let schedule = schedule(&app, &token, &model, json!({ "projectId": project })).await;
    // The model calls the tool anyway: it is not in the run's tools, so it never runs.
    let arguments = json!({
        "name": "More",
        "cron": "0 * * * *",
        "timezone": "UTC",
        "prompt": "Again.",
    });
    script(
        &provider,
        calls(&[("schedule_prompt", &arguments.to_string())]),
    )
    .await;
    script(&provider, answer("I cannot schedule from here.")).await;

    let started = Scheduler::new(state.clone())
        .tick(due(&schedule) + second())
        .await;
    quiet(&state).await;
    let chat = started[0].to_string();
    let sent = completions(&provider).await;
    assert_eq!(sent.len(), 2);
    for request in &sent {
        assert!(
            request.get("tools").is_none(),
            "no schedule tools in an unattended run: {request}"
        );
    }
    let saved = messages(&app, &token, &chat).await;
    let result = &saved[2]["parts"][0];
    assert_eq!(result["status"], "error");
    assert_eq!(
        result["content"],
        json!([{ "type": "text", "text": "Unknown tool." }])
    );
    assert_eq!(content(&saved[3]), "I cannot schedule from here.");
    let listed = call(&app, &token, Method::GET, "/api/schedules", None).await;
    assert_eq!(
        listed["items"].as_array().map(Vec::len),
        Some(1),
        "the run made no schedule"
    );

    // An interactive chat in the same project is offered them.
    let created = call(
        &app,
        &token,
        Method::POST,
        "/api/chats",
        Some(json!({ "modelId": model, "projectId": project })),
    )
    .await;
    let chat = created["data"]["id"].as_str().expect("an id").to_owned();
    script(&provider, answer("Hi.")).await;
    say(&app, &token, &chat, "Hello").await;
    let sent = completions(&provider).await;
    let tools: Vec<&str> = sent[2]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|tool| tool["function"]["name"].as_str().expect("a name"))
        .collect();
    assert_eq!(
        tools,
        ["cancel_schedule", "list_schedules", "schedule_prompt"]
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn three_failed_runs_in_a_row_turn_the_schedule_off(pool: DbPool) {
    let (app, token, model, provider) = start(pool).await;
    let state = app.state.clone();
    let app = app.router;
    let rejected = || {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": { "message": "Incorrect API key provided.", "type": "invalid_request_error" }
            })))
    };
    // Two failures, a success, then failures only.
    rejected().up_to_n_times(2).mount(&provider).await;
    script(&provider, answer("Fine again.")).await;
    rejected().mount(&provider).await;
    let schedule = schedule(&app, &token, &model, json!({})).await;
    let scheduler = Scheduler::new(state.clone());

    let mut now = due(&schedule) + second();
    for failures in [1, 2, 0, 1, 2, 3] {
        assert_eq!(scheduler.tick(now).await.len(), 1, "fires while on");
        quiet(&state).await;
        let stored = read(&app, &token, &schedule).await;
        assert_eq!(stored["consecutiveFailures"], failures);
        if failures == 0 {
            assert_eq!(stored["lastStatus"], "ok");
            assert_eq!(stored["lastError"], Value::Null);
        } else {
            assert_eq!(stored["lastStatus"], "error");
            assert_eq!(stored["lastError"], "Key rejected: open provider settings.");
        }
        assert_eq!(stored["enabled"], failures < 3);
        if failures < 3 {
            now = due(&stored) + second();
        } else {
            assert_eq!(stored["disabledReason"], "Failed 3 times in a row");
            assert_eq!(stored["nextRunAt"], Value::Null);
        }
    }
    let far = now + chrono::Duration::days(30);
    assert!(
        scheduler.tick(far).await.is_empty(),
        "a schedule that is off never fires"
    );
    assert_eq!(runs(&app, &token, &schedule).await.len(), 6);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn a_schedule_whose_model_is_gone_pauses_instead_of_failing(pool: DbPool) {
    let (app, token, model, provider) = start(pool.clone()).await;
    let state = app.state.clone();
    let app = app.router;
    let schedule = schedule(&app, &token, &model, json!({})).await;
    let id = schedule["id"].as_str().expect("an id").to_owned();
    // The provider stopped listing the model (a refresh marks it unavailable).
    sqlx::query("UPDATE models SET available = false")
        .execute(&pool)
        .await
        .expect("the model is gone");

    let mut heard = events(&app, &token).await;
    let now = due(&schedule) + second();
    assert!(Scheduler::new(state.clone()).tick(now).await.is_empty());
    quiet(&state).await;
    let stored = read(&app, &token, &schedule).await;
    assert_eq!(stored["enabled"], false);
    assert_eq!(stored["disabledReason"], "Model removed");
    assert_eq!(stored["nextRunAt"], Value::Null);
    assert_eq!(stored["lastStatus"], "error");
    assert!(
        stored["lastError"]
            .as_str()
            .is_some_and(|error| error.contains("no longer offered")),
        "{stored}"
    );
    assert!(
        runs(&app, &token, &schedule).await.is_empty(),
        "no empty chat"
    );
    assert!(completions(&provider).await.is_empty());
    let updated = json!({ "type": "schedule.updated", "scheduleId": id });
    assert_eq!(heard.next().await, Some(updated.clone()), "the claim");
    assert_eq!(heard.next().await, Some(updated), "the pause");

    // Run now says why.
    let refused = send(
        app.clone(),
        request(
            Method::POST,
            &format!("/api/schedules/{id}/run"),
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused.body["code"], "MODEL_UNAVAILABLE");
    assert_eq!(refused.body["error"], stored["lastError"]);
}

/// `POST /schedules/{id}/run` of `schedule` as `token`'s user.
async fn run_now(app: &Router, token: &str, schedule: &Value) -> testing::TestResponse {
    let id = schedule["id"].as_str().expect("an id");
    send(
        app.clone(),
        request(
            Method::POST,
            &format!("/api/schedules/{id}/run"),
            Some(token),
            None,
        ),
    )
    .await
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn one_users_runs_never_keep_another_users_due_schedule_waiting(pool: DbPool) {
    let llm = LlmServer::start().await;
    let mut open = config();
    // Bob is no administrator, and the scripted provider listens on loopback.
    open.private_network = PrivateNetwork::All;
    let app = live(pool, open).await;
    let state = app.state.clone();
    let app = app.router;
    let alice = testing::register(&app, "admin").await;
    let alices_model = testing::model_at(&app, &alice, &llm.url).await;
    let bob = other_user(&app, &alice, "bob").await;
    let bobs_model = testing::model_at(&app, &bob, &llm.url).await;
    let mine = schedule(&app, &alice, &alices_model, json!({ "name": "Mine" })).await;
    let also_mine = schedule(&app, &alice, &alices_model, json!({ "name": "Also mine" })).await;
    let theirs = schedule(&app, &bob, &bobs_model, json!({ "name": "Theirs" })).await;
    let writers = [llm.live(), llm.live()];

    // Alice's Run now holds a slot for as long as her provider keeps answering, and a second
    // one would take the other slot.
    let started = run_now(&app, &alice, &mine).await;
    assert_eq!(started.status, StatusCode::OK, "{}", started.body);
    requested(&llm, 1).await;
    let again = run_now(&app, &alice, &also_mine).await;
    assert_eq!(again.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(again.body["code"], "SCHEDULER_BUSY");
    assert_eq!(
        again.body["error"],
        "One of your schedules is running. Try again when it finishes."
    );

    // Her other schedule and Bob's come due together: Bob's gets the free slot.
    let scheduler = Scheduler::new(state.clone());
    let now = due(&theirs).max(due(&also_mine)) + second();
    assert_eq!(scheduler.tick(now).await.len(), 1, "one free slot");
    assert_eq!(
        runs(&app, &bob, &theirs).await.len(),
        1,
        "Bob's schedule ran"
    );
    assert!(runs(&app, &alice, &also_mine).await.is_empty());
    requested(&llm, 2).await;
    for writer in writers {
        writer.text("Done.");
        writer.finish("stop");
    }
    quiet(&state).await;

    // Hers (Run now kept the first one due too) fire at the next tick, with both slots free.
    let waiting = [llm.live(), llm.live()];
    let later = now + chrono::Duration::seconds(30);
    assert_eq!(scheduler.tick(later).await.len(), 2);
    requested(&llm, 4).await;
    for writer in waiting {
        writer.text("Done.");
        writer.finish("stop");
    }
    quiet(&state).await;
    assert_eq!(runs(&app, &alice, &also_mine).await.len(), 1);
    assert_eq!(runs(&app, &alice, &mine).await.len(), 2);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn run_now_starts_an_unattended_run_and_keeps_the_next_run(pool: DbPool) {
    let (app, token, model, provider) = start(pool).await;
    let state = app.state.clone();
    let app = app.router;
    let schedule = schedule(&app, &token, &model, json!({})).await;
    let id = schedule["id"].as_str().expect("an id").to_owned();
    let bob = other_user(&app, &token, "bob").await;
    let run = |token: String, id: String| {
        let app = app.clone();
        async move {
            send(
                app,
                request(
                    Method::POST,
                    &format!("/api/schedules/{id}/run"),
                    Some(&token),
                    None,
                ),
            )
            .await
        }
    };

    let theirs = run(bob, id.clone()).await;
    assert_eq!(
        theirs.status,
        StatusCode::NOT_FOUND,
        "another user's schedule"
    );
    let malformed = run(token.clone(), "not-a-uuid".to_owned()).await;
    assert_eq!(malformed.status, StatusCode::BAD_REQUEST);
    assert_eq!(malformed.body["field"], "id");

    next(&provider, answer("Here it is."), Duration::from_millis(300)).await;
    let started = run(token.clone(), id.clone()).await;
    assert_eq!(started.status, StatusCode::OK, "{}", started.body);
    let chat = started.body["data"]["chatId"]
        .as_str()
        .expect("the run's chat")
        .to_owned();
    let mut stream = attach(&app, &token, &chat, None)
        .await
        .expect("the run is live");
    assert_eq!(text(&stream.rest().await), "Here it is.");
    quiet(&state).await;

    let stored = read(&app, &token, &schedule).await;
    assert_eq!(stored["lastStatus"], "ok");
    assert_eq!(
        stored["nextRunAt"], schedule["nextRunAt"],
        "Run now does not move the next run"
    );
    let listed = runs(&app, &token, &schedule).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["id"], chat.as_str());
    let request = &completions(&provider).await[0];
    assert!(
        request.get("tools").is_none(),
        "unattended, like a scheduled fire"
    );
}
