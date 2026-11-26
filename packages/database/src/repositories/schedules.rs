//! `ScheduleRepository` on Postgres (`schedules`). Roadmap step P2.A writes the queries; `due`
//! is the one place that reads `auth.users` (`disabled_at`, `role`).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use copilot_core::{
    Actor, DomainResult, ModelId, RunRecord, Schedule, ScheduleId, ScheduleQuery,
    ScheduleRepository, UserId,
};

use crate::repositories::store::{PgStore, pending};

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
        _user: UserId,
        _model: ModelId,
        _reason: &str,
    ) -> DomainResult<u64> {
        pending("ScheduleRepository::disable_for_model")
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
