//! Schedule persistence port.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domains::schedule::entities::{CreatedBy, RunRecord, Schedule};
use crate::shared::actor::Actor;
use crate::shared::errors::DomainResult;
use crate::shared::ids::{ChatId, ModelId, ProjectId, ScheduleId, UserId};

#[async_trait]
pub trait ScheduleRepository: Send + Sync {
    /// Oldest first.
    async fn list(&self, user: UserId, query: ScheduleQuery) -> DomainResult<Vec<Schedule>>;
    async fn get(&self, user: UserId, id: ScheduleId) -> DomainResult<Option<Schedule>>;
    async fn insert(&self, schedule: &Schedule) -> DomainResult<()>;
    /// Writes the user's settings and `enabled`, `next_run_at`, `consecutive_failures`, and
    /// `disabled_reason`. The run columns (`last_run_at`, `last_status`, `last_error`) belong to
    /// `advance` and `record`.
    async fn update(&self, schedule: &Schedule) -> DomainResult<()>;
    /// `false` when the user has no such schedule. Chats of its runs keep existing
    /// (`schedule_id` becomes null).
    async fn delete(&self, user: UserId, id: ScheduleId) -> DomainResult<bool>;
    async fn count_enabled(&self, user: UserId) -> DomainResult<u32>;
    /// Pauses the user's enabled schedules that run `model`: `enabled = false`,
    /// `next_run_at = NULL`, `disabled_reason = reason`. Returns how many.
    async fn disable_for_model(
        &self,
        user: UserId,
        model: ModelId,
        reason: &str,
    ) -> DomainResult<u64>;
    /// System: enabled schedules with `next_run_at <= now`, oldest due first, at most `limit`,
    /// each with its owner's `Actor`. Joins `auth.users`: owners with `disabled_at` set are
    /// skipped, and `is_admin = (role = 'administrator')`. Takes no lock; claim each one with
    /// [`ScheduleRepository::advance`].
    async fn due(&self, now: DateTime<Utc>, limit: u32) -> DomainResult<Vec<(Schedule, Actor)>>;
    /// System compare-and-set claim: `next_run_at = next, last_run_at = now` only while the
    /// schedule is enabled and `next_run_at = expected`. `true` when this caller claimed it.
    async fn advance(
        &self,
        id: ScheduleId,
        expected: DateTime<Utc>,
        next: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> DomainResult<bool>;
    /// System: stores how a run ended (see `RunRecord`), in one statement.
    async fn record(&self, id: ScheduleId, outcome: &RunRecord) -> DomainResult<()>;
}

/// Which schedules to list; the filters combine.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScheduleQuery {
    /// Only schedules in this project.
    pub project: Option<ProjectId>,
    /// Only schedules created from this chat.
    pub source_chat: Option<ChatId>,
    pub created_by: Option<CreatedBy>,
}
