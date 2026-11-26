use chrono::{DateTime, Utc};
use copilot_core::{ScheduleRepository, UserId};
use copilot_database::{DbPool, PgStore};
use uuid::Uuid;

use crate::{NOW, at, model, store, user};

/// A schedule of `owner` on `model` (`ScheduleRepository::insert` is P2.A's), enabled with a
/// next run time or already turned off.
async fn schedule(store: &PgStore, owner: UserId, model: Uuid, enabled: bool) -> Uuid {
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
