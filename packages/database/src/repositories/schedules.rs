//! `ScheduleRepository` on Postgres (`schedules`). Roadmap step P2.A writes the queries; `due`
//! is the one place that reads `auth.users` (`disabled_at`, `role`). `disable_for_model` is
//! written already, because turning a model off (`PATCH /models/{id}`) calls it.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use copilot_core::{
    Actor, DomainResult, ModelId, RunRecord, Schedule, ScheduleId, ScheduleQuery,
    ScheduleRepository, UserId,
};

use crate::repositories::store::{PgStore, failed, pending};

#[async_trait]
impl ScheduleRepository for PgStore {
    async fn list(&self, _user: UserId, _query: ScheduleQuery) -> DomainResult<Vec<Schedule>> {
        pending("ScheduleRepository::list")
    }

    async fn get(&self, _user: UserId, _id: ScheduleId) -> DomainResult<Option<Schedule>> {
        pending("ScheduleRepository::get")
    }

    async fn insert(&self, _schedule: &Schedule) -> DomainResult<()> {
        pending("ScheduleRepository::insert")
    }

    async fn update(&self, _schedule: &Schedule) -> DomainResult<()> {
        pending("ScheduleRepository::update")
    }

    async fn reschedule(&self, _schedule: &Schedule, _read: &Schedule) -> DomainResult<bool> {
        pending("ScheduleRepository::reschedule")
    }

    async fn delete(&self, _user: UserId, _id: ScheduleId) -> DomainResult<bool> {
        pending("ScheduleRepository::delete")
    }

    async fn count_enabled(&self, _user: UserId) -> DomainResult<u32> {
        pending("ScheduleRepository::count_enabled")
    }

    async fn disable_for_model(
        &self,
        user: UserId,
        model: ModelId,
        reason: &str,
    ) -> DomainResult<u64> {
        let disabled = sqlx::query(
            "UPDATE schedules SET enabled = false, next_run_at = NULL, disabled_reason = $3, \
             updated_at = $4 WHERE user_id = $1 AND model_id = $2 AND enabled",
        )
        .bind(user.uuid())
        .bind(model.uuid())
        .bind(reason)
        .bind(self.now())
        .execute(self.pool())
        .await
        .map_err(failed("database.schedules.disable.failed"))?
        .rows_affected();
        Ok(disabled)
    }

    async fn due(&self, _now: DateTime<Utc>, _limit: u32) -> DomainResult<Vec<(Schedule, Actor)>> {
        pending("ScheduleRepository::due")
    }

    async fn advance(
        &self,
        _id: ScheduleId,
        _expected: DateTime<Utc>,
        _next: DateTime<Utc>,
        _now: DateTime<Utc>,
    ) -> DomainResult<bool> {
        pending("ScheduleRepository::advance")
    }

    async fn record(&self, _id: ScheduleId, _outcome: &RunRecord) -> DomainResult<()> {
        pending("ScheduleRepository::record")
    }
}
