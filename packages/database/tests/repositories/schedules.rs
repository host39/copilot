use chrono::{DateTime, Utc};
use copilot_core::{
    Actor, Chat, ChatRepository, CreatedBy, CronRule, OnAsk, Project, ProjectRepository,
    ReasoningLevel, RunOutcome, RunRecord, RunStatus, Schedule, ScheduleParts, ScheduleQuery,
    ScheduleRepository, UserId,
};
use copilot_database::{DbPool, PgStore};
use uuid::Uuid;

use crate::{NOW, at, model, store, user};

/// A schedule of `owner` on `model` written with SQL, enabled with a next run time or already
/// turned off.
pub async fn schedule(store: &PgStore, owner: UserId, model: Uuid, enabled: bool) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO schedules (created_at, updated_at, next_run_at, id, user_id, model_id, \
         enabled, name, cron, timezone, prompt, created_by) \
         VALUES ($1, $1, $2, $3, $4, $5, $6, 'Daily', '0 9 * * *', 'UTC', 'Summarize.', 'user')",
    )
    .bind(at(0))
    .bind(enabled.then(|| at(60)))
    .bind(id)
    .bind(owner.uuid())
    .bind(model)
    .bind(enabled)
    .execute(store.pool())
    .await
    .expect("schedule inserted");
    id
}

/// Moves the next run of schedule `id` to `when`.
async fn due_at(store: &PgStore, id: Uuid, when: DateTime<Utc>) {
    sqlx::query("UPDATE schedules SET next_run_at = $2 WHERE id = $1")
        .bind(id)
        .bind(when)
        .execute(store.pool())
        .await
        .expect("schedule moved");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn due_schedules_take_turns_between_owners_and_busy_owners_wait(pool: DbPool) {
    let store = store(pool).await;
    let alice = user(&store).await;
    let bob = user(&store).await;
    let alices_model = model(&store, alice).await.id().uuid();
    let bobs_model = model(&store, bob).await.id().uuid();
    // Alice's three schedules all came due before Bob's.
    let mut alices = Vec::new();
    for seconds in [1, 2, 3] {
        let id = schedule(&store, alice, alices_model, true).await;
        due_at(&store, id, at(seconds)).await;
        alices.push(id);
    }
    let bobs = schedule(&store, bob, bobs_model, true).await;
    due_at(&store, bobs, at(10)).await;
    let order = |due: Vec<(Schedule, Actor)>| -> Vec<Uuid> {
        due.iter()
            .map(|(schedule, _)| schedule.id().uuid())
            .collect()
    };

    assert_eq!(
        order(store.due(at(20), 10, &[]).await.unwrap()),
        [alices[0], bobs, alices[1], alices[2]],
        "each owner's oldest before anyone's second"
    );
    assert_eq!(
        order(store.due(at(20), 2, &[]).await.unwrap()),
        [alices[0], bobs]
    );
    assert_eq!(
        order(store.due(at(20), 2, &[alice]).await.unwrap()),
        [bobs, alices[0]],
        "an owner holding a slot waits for everyone else"
    );
    assert_eq!(
        order(store.due(at(20), 10, &[alice, bob]).await.unwrap()),
        [alices[0], bobs, alices[1], alices[2]],
        "turns among busy owners too"
    );
}

#[derive(Debug, PartialEq, sqlx::FromRow)]
struct State {
    enabled: bool,
    next_run_at: Option<DateTime<Utc>>,
    disabled_reason: Option<String>,
    updated_at: DateTime<Utc>,
}

async fn state(store: &PgStore, id: Uuid) -> State {
    sqlx::query_as(
        "SELECT enabled, next_run_at, disabled_reason, updated_at FROM schedules WHERE id = $1",
    )
    .bind(id)
    .fetch_one(store.pool())
    .await
    .expect("schedule exists")
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn turning_a_model_off_disables_only_its_enabled_schedules(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let turned_off = model(&store, owner).await;
    let kept = model(&store, owner).await;
    let foreign = model(&store, other).await;
    let running = schedule(&store, owner, turned_off.id().uuid(), true).await;
    let stopped = schedule(&store, owner, turned_off.id().uuid(), false).await;
    let elsewhere = schedule(&store, owner, kept.id().uuid(), true).await;
    let theirs = schedule(&store, other, foreign.id().uuid(), true).await;

    let count = store
        .disable_for_model(other, turned_off.id(), "Model turned off")
        .await
        .unwrap();
    assert_eq!(count, 0, "another user's call changes nothing");

    let count = store
        .disable_for_model(owner, turned_off.id(), "Model turned off")
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        state(&store, running).await,
        State {
            enabled: false,
            next_run_at: None,
            disabled_reason: Some("Model turned off".to_owned()),
            updated_at: at(NOW),
        }
    );
    let untouched = |next_run_at: Option<DateTime<Utc>>| State {
        enabled: next_run_at.is_some(),
        next_run_at,
        disabled_reason: None,
        updated_at: at(0),
    };
    assert_eq!(state(&store, stopped).await, untouched(None));
    assert_eq!(state(&store, elsewhere).await, untouched(Some(at(60))));
    assert_eq!(state(&store, theirs).await, untouched(Some(at(60))));
}

/// A daily schedule of `owner` on `model` at 09:00 UTC, created at `at(0)`.
fn daily(owner: UserId, model: &copilot_core::Model) -> Schedule {
    let rule = CronRule::parse("0 9 * * *", "UTC", at(0)).unwrap();
    Schedule::create(owner, "Daily", rule, "Summarize.", at(0))
        .unwrap()
        .with_model(Some(model.id()))
}

/// The schedule as stored.
async fn stored(store: &PgStore, owner: UserId, id: copilot_core::ScheduleId) -> Schedule {
    ScheduleRepository::get(store, owner, id)
        .await
        .unwrap()
        .expect("the schedule exists")
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn schedules_round_trip_and_list_by_target(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let model = model(&store, owner).await;
    let project = Project::create(owner, "Research", at(0)).unwrap();
    ProjectRepository::insert(&store, &project).await.unwrap();
    let chat = Chat::create(owner, at(0)).with_project(Some(project.id()));
    ChatRepository::insert(&store, &chat).await.unwrap();

    let standalone = daily(owner, &model)
        .with_instructions("Be brief.")
        .with_reasoning(Some(ReasoningLevel::Low));
    ScheduleRepository::insert(&store, &standalone)
        .await
        .unwrap();
    let rule = CronRule::parse("30 7 * * 1-5", "Europe/Berlin", at(0)).unwrap();
    let assistant = Schedule::create(owner, "Weekday news", rule, "News.", at(1))
        .unwrap()
        .in_project(Some(project.id()))
        .with_model(Some(model.id()))
        .with_on_ask(OnAsk::Pause)
        .by_assistant(chat.id())
        .disabled(Some("Paused".to_owned()));
    ScheduleRepository::insert(&store, &assistant)
        .await
        .unwrap();

    assert_eq!(stored(&store, owner, standalone.id()).await, standalone);
    assert_eq!(stored(&store, owner, assistant.id()).await, assistant);
    assert_eq!(
        ScheduleRepository::get(&store, other, standalone.id())
            .await
            .unwrap(),
        None
    );
    let list = |query: ScheduleQuery| {
        let store = store.clone();
        async move {
            ScheduleRepository::list(&store, owner, query)
                .await
                .unwrap()
                .iter()
                .map(Schedule::id)
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        list(ScheduleQuery::default()).await,
        vec![standalone.id(), assistant.id()],
        "oldest first"
    );
    let in_project = ScheduleQuery {
        project: Some(project.id()),
        ..ScheduleQuery::default()
    };
    assert_eq!(list(in_project).await, vec![assistant.id()]);
    let from_chat = ScheduleQuery {
        source_chat: Some(chat.id()),
        created_by: Some(CreatedBy::Assistant),
        ..ScheduleQuery::default()
    };
    assert_eq!(list(from_chat).await, vec![assistant.id()]);
    let by_user = ScheduleQuery {
        created_by: Some(CreatedBy::User),
        ..ScheduleQuery::default()
    };
    assert_eq!(list(by_user).await, vec![standalone.id()]);
    assert!(
        ScheduleRepository::list(&store, other, ScheduleQuery::default())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.count_enabled(owner).await.unwrap(), 1);
    assert_eq!(store.count_enabled(other).await.unwrap(), 0);

    assert!(
        !ScheduleRepository::delete(&store, other, standalone.id())
            .await
            .unwrap()
    );
    assert!(
        ScheduleRepository::delete(&store, owner, standalone.id())
            .await
            .unwrap()
    );
    assert_eq!(list(ScheduleQuery::default()).await, vec![assistant.id()]);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn settings_writes_never_touch_the_rule_or_the_schedulers_columns(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let model = model(&store, owner).await;
    let other_model = crate::model(&store, owner).await;
    let schedule = daily(owner, &model);
    ScheduleRepository::insert(&store, &schedule).await.unwrap();
    let due = schedule.next_run_at().unwrap();

    // The scheduler claims a fire while the user edits the settings of the copy they read.
    assert!(
        store
            .advance(schedule.id(), due, at(NOW), at(5))
            .await
            .unwrap()
    );
    let rule = CronRule::parse("0 12 * * *", "UTC", at(0)).unwrap();
    let edited = schedule
        .clone()
        .with_name("Noon")
        .unwrap()
        .with_prompt("Lunch ideas.")
        .unwrap()
        .with_instructions("Short.")
        .with_model(Some(other_model.id()))
        .with_reasoning(Some(ReasoningLevel::High))
        .with_on_ask(OnAsk::Pause)
        .with_rule(rule, at(6))
        .disabled(None)
        .touched(at(6));
    ScheduleRepository::update(&store, &edited).await.unwrap();
    let after = stored(&store, owner, schedule.id()).await;
    assert_eq!(after.name(), "Noon");
    assert_eq!(after.prompt(), "Lunch ideas.");
    assert_eq!(after.instructions(), "Short.");
    assert_eq!(after.model_id(), Some(other_model.id()));
    assert_eq!(after.reasoning(), Some(ReasoningLevel::High));
    assert_eq!(after.on_ask(), OnAsk::Pause);
    assert_eq!(after.updated_at(), at(6));
    assert_eq!(after.rule().expression(), "0 9 * * *", "the rule stays");
    assert!(after.enabled(), "the on/off state stays");
    assert_eq!(after.next_run_at(), Some(at(NOW)), "the claim stays");
    assert_eq!(after.last_run_at(), Some(at(5)));

    let foreign = Schedule::restore(ScheduleParts {
        user_id: other,
        name: "Taken".to_owned(),
        ..parts(&after)
    });
    ScheduleRepository::update(&store, &foreign).await.unwrap();
    assert_eq!(stored(&store, owner, schedule.id()).await.name(), "Noon");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn reschedule_writes_only_while_the_schedulers_columns_are_unchanged(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let model = model(&store, owner).await;
    let schedule = daily(owner, &model);
    ScheduleRepository::insert(&store, &schedule).await.unwrap();
    let read = stored(&store, owner, schedule.id()).await;
    let rule = CronRule::parse("0 12 * * *", "Europe/Berlin", at(0)).unwrap();
    let moved = read.clone().with_rule(rule, at(7)).touched(at(7));

    // A claim since the read: the new rule is not written.
    let due = read.next_run_at().unwrap();
    assert!(store.advance(read.id(), due, at(NOW), at(6)).await.unwrap());
    assert!(!store.reschedule(&moved, &read).await.unwrap());
    let current = stored(&store, owner, read.id()).await;
    assert_eq!(current.rule().expression(), "0 9 * * *");
    assert_eq!(current.next_run_at(), Some(at(NOW)));

    // Applied to a fresh read, it is.
    let rule = CronRule::parse("0 12 * * *", "Europe/Berlin", at(0)).unwrap();
    let moved = current
        .clone()
        .with_name("Noon")
        .unwrap()
        .with_rule(rule, at(8))
        .touched(at(8));
    assert!(store.reschedule(&moved, &current).await.unwrap());
    let after = stored(&store, owner, read.id()).await;
    assert_eq!(
        after,
        moved.clone().advanced(moved.next_run_at().unwrap(), at(6))
    );
    assert_eq!(after.rule().timezone(), "Europe/Berlin");
    assert_eq!(after.last_run_at(), Some(at(6)), "the run columns stay");

    // Off, then on again (clearing the failures and the reason), each against its read.
    let off = after.clone().disabled(None).touched(at(9));
    assert!(store.reschedule(&off, &after).await.unwrap());
    let off = stored(&store, owner, read.id()).await;
    assert!(!off.enabled());
    assert_eq!(off.next_run_at(), None);
    assert!(
        !store.reschedule(&off, &after).await.unwrap(),
        "a stale read"
    );
    let on = off.clone().enabled_at(at(10)).touched(at(10));
    assert!(store.reschedule(&on, &off).await.unwrap());
    assert_eq!(
        stored(&store, owner, read.id()).await.next_run_at(),
        on.next_run_at()
    );

    let mut foreign = parts(&on);
    foreign.user_id = user(&store).await;
    let foreign = Schedule::restore(foreign).disabled(None);
    assert!(
        !store.reschedule(&foreign, &on).await.unwrap(),
        "another user's"
    );
    assert!(stored(&store, owner, read.id()).await.enabled());
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn concurrent_claims_fire_a_due_schedule_once(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let model = model(&store, owner).await;
    let schedule = daily(owner, &model);
    ScheduleRepository::insert(&store, &schedule).await.unwrap();
    let due = schedule.next_run_at().unwrap();
    let next = schedule.rule().next_after(due).unwrap();

    let (first, second) = tokio::join!(
        store.advance(schedule.id(), due, next, at(NOW)),
        store.advance(schedule.id(), due, next, at(NOW)),
    );
    let claims = [first.unwrap(), second.unwrap()];
    assert_eq!(
        claims.iter().filter(|claimed| **claimed).count(),
        1,
        "{claims:?}"
    );
    let after = stored(&store, owner, schedule.id()).await;
    assert_eq!(after.next_run_at(), Some(next));
    assert_eq!(after.last_run_at(), Some(at(NOW)));
    assert!(
        !store
            .advance(schedule.id(), due, next, at(NOW))
            .await
            .unwrap(),
        "the fire was claimed"
    );

    let off = after.clone().disabled(None);
    assert!(store.reschedule(&off, &after).await.unwrap());
    assert!(
        !store
            .advance(schedule.id(), next, next, at(NOW))
            .await
            .unwrap(),
        "a disabled schedule is never claimed"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn due_skips_disabled_owners_and_builds_each_owners_actor(pool: DbPool) {
    let store = store(pool).await;
    let admin = user(&store).await;
    let plain = user(&store).await;
    let manager = user(&store).await;
    let disabled = user(&store).await;
    for (owner, role) in [(admin, "administrator"), (manager, "manager")] {
        sqlx::query("UPDATE auth.users SET role = $2 WHERE id = $1")
            .bind(owner.uuid())
            .bind(role)
            .execute(store.pool())
            .await
            .unwrap();
    }
    sqlx::query("UPDATE auth.users SET disabled_at = now() WHERE id = $1")
        .bind(disabled.uuid())
        .execute(store.pool())
        .await
        .unwrap();

    let mut ids = Vec::new();
    for owner in [admin, plain, manager, disabled] {
        let model = model(&store, owner).await;
        let schedule = daily(owner, &model);
        ScheduleRepository::insert(&store, &schedule).await.unwrap();
        ids.push(schedule.id());
    }
    let model = model(&store, plain).await;
    let off = daily(plain, &model).disabled(None);
    ScheduleRepository::insert(&store, &off).await.unwrap();
    let later = Schedule::create(
        plain,
        "Later",
        CronRule::parse("0 9 1 1 *", "UTC", at(0)).unwrap(),
        "Happy new year.",
        at(0),
    )
    .unwrap();
    ScheduleRepository::insert(&store, &later).await.unwrap();

    let due_at = stored(&store, admin, ids[0]).await.next_run_at().unwrap();
    assert!(
        store
            .due(due_at - chrono::Duration::seconds(1), 10, &[])
            .await
            .unwrap()
            .is_empty(),
        "nothing is due before its time"
    );
    let due = store.due(due_at, 10, &[]).await.unwrap();
    let found: Vec<_> = due
        .iter()
        .map(|(schedule, actor)| (schedule.id(), *actor))
        .collect();
    let mut expected = vec![
        (ids[0], Actor::admin(admin)),
        (ids[1], Actor::user(plain)),
        (ids[2], Actor::user(manager)),
    ];
    expected.sort_by_key(|(id, _)| id.uuid());
    assert_eq!(
        found, expected,
        "same time: by id; no disabled owner or schedule"
    );
    assert_eq!(
        due[0].0,
        stored(&store, due[0].1.user_id, due[0].0.id()).await
    );
    assert_eq!(
        store.due(due_at, 2, &[]).await.unwrap().len(),
        2,
        "at most the limit"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn run_records_count_failures_and_disable_after_the_third(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let model = model(&store, owner).await;
    let schedule = daily(owner, &model);
    ScheduleRepository::insert(&store, &schedule).await.unwrap();
    let mut expected = schedule.clone();
    let outcomes = [
        RunOutcome::Failed("Provider down.".to_owned()),
        RunOutcome::Ok,
        RunOutcome::Failed("One.".to_owned()),
        RunOutcome::AwaitingApproval,
        RunOutcome::Failed("Two.".to_owned()),
        RunOutcome::Failed("Three.".to_owned()),
        RunOutcome::Failed("Four.".to_owned()),
    ];
    for (index, outcome) in outcomes.into_iter().enumerate() {
        let record = RunRecord::new(outcome, at(10 + index as i64));
        store.record(schedule.id(), &record).await.unwrap();
        expected = expected.recorded(&record);
        assert_eq!(
            stored(&store, owner, schedule.id()).await,
            expected,
            "after run {index}"
        );
    }
    let after = stored(&store, owner, schedule.id()).await;
    assert!(!after.enabled());
    assert_eq!(after.next_run_at(), None);
    assert_eq!(after.disabled_reason(), Some("Failed 3 times in a row"));
    assert_eq!(after.consecutive_failures(), 4, "counted on while off");
    assert_eq!(after.last_status(), Some(RunStatus::Error));
    assert_eq!(after.last_error(), Some("Four."));
}

/// Every field of `schedule`, to build a changed copy.
fn parts(schedule: &Schedule) -> ScheduleParts {
    ScheduleParts {
        id: schedule.id(),
        user_id: schedule.user_id(),
        project_id: schedule.project_id(),
        model_id: schedule.model_id(),
        source_chat_id: schedule.source_chat_id(),
        name: schedule.name().to_owned(),
        rule: schedule.rule().clone(),
        prompt: schedule.prompt().to_owned(),
        instructions: schedule.instructions().to_owned(),
        reasoning: schedule.reasoning(),
        on_ask: schedule.on_ask(),
        created_by: schedule.created_by(),
        enabled: schedule.enabled(),
        next_run_at: schedule.next_run_at(),
        last_run_at: schedule.last_run_at(),
        last_status: schedule.last_status(),
        last_error: schedule.last_error().map(str::to_owned),
        consecutive_failures: schedule.consecutive_failures(),
        disabled_reason: schedule.disabled_reason().map(str::to_owned),
        created_at: schedule.created_at(),
        updated_at: schedule.updated_at(),
    }
}
