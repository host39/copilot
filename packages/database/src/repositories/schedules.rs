//! `ScheduleRepository` on Postgres (`schedules`). `due` is the one place that reads `auth.users`
//! (`disabled_at`, `role`). User writes and the scheduler's writes never undo each other:
//! `update` writes settings only, `reschedule` writes the rule and the on/off state only while
//! the scheduler's columns still hold what the caller read, `advance` claims a fire by
//! compare-and-set, and `record` applies a run's outcome in one statement.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use copilot_core::{
    Actor, ChatId, CronRule, DomainError, DomainResult, ModelId, ProjectId, ReasoningLevel,
    RunRecord, Schedule, ScheduleId, ScheduleParts, ScheduleQuery, ScheduleRepository, UserId,
};
use uuid::Uuid;

use crate::repositories::store::{PgStore, failed, text};

const COLUMNS: &str = "s.created_at, s.updated_at, s.next_run_at, s.last_run_at, \
     s.consecutive_failures, s.id, s.user_id, s.project_id, s.model_id, s.source_chat_id, \
     s.enabled, s.name, s.cron, s.timezone, s.prompt, s.instructions, s.reasoning, s.on_ask, \
     s.created_by, s.last_status, s.last_error, s.disabled_reason";

#[derive(sqlx::FromRow)]
struct ScheduleRow {
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    next_run_at: Option<DateTime<Utc>>,
    last_run_at: Option<DateTime<Utc>>,
    consecutive_failures: i32,
    id: Uuid,
    user_id: Uuid,
    project_id: Option<Uuid>,
    model_id: Option<Uuid>,
    source_chat_id: Option<Uuid>,
    enabled: bool,
    name: String,
    cron: String,
    timezone: String,
    prompt: String,
    instructions: String,
    reasoning: Option<String>,
    on_ask: String,
    created_by: String,
    last_status: Option<String>,
    last_error: Option<String>,
    disabled_reason: Option<String>,
}

/// A due schedule with its owner's role.
#[derive(sqlx::FromRow)]
struct DueRow {
    #[sqlx(flatten)]
    schedule: ScheduleRow,
    is_admin: bool,
}

impl TryFrom<ScheduleRow> for Schedule {
    type Error = DomainError;

    fn try_from(row: ScheduleRow) -> DomainResult<Self> {
        Ok(Self::restore(ScheduleParts {
            id: ScheduleId::new(row.id),
            user_id: UserId::new(row.user_id),
            project_id: row.project_id.map(ProjectId::new),
            model_id: row.model_id.map(ModelId::new),
            source_chat_id: row.source_chat_id.map(ChatId::new),
            name: row.name,
            rule: CronRule::restore(&row.cron, &row.timezone)?,
            prompt: row.prompt,
            instructions: row.instructions,
            reasoning: row
                .reasoning
                .map(|level| text(&level, "database.schedules.reasoning.invalid"))
                .transpose()?,
            on_ask: text(&row.on_ask, "database.schedules.on-ask.invalid")?,
            created_by: text(&row.created_by, "database.schedules.created-by.invalid")?,
            enabled: row.enabled,
            next_run_at: row.next_run_at,
            last_run_at: row.last_run_at,
            last_status: row
                .last_status
                .map(|status| text(&status, "database.schedules.last-status.invalid"))
                .transpose()?,
            last_error: row.last_error,
            // `CHECK (consecutive_failures >= 0)` holds it.
            consecutive_failures: u32::try_from(row.consecutive_failures).unwrap_or(0),
            disabled_reason: row.disabled_reason,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }))
    }
}

/// An `integer` count column.
fn count(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

#[async_trait]
impl ScheduleRepository for PgStore {
    async fn list(&self, user: UserId, query: ScheduleQuery) -> DomainResult<Vec<Schedule>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM schedules s WHERE s.user_id = $1 \
             AND ($2::uuid IS NULL OR s.project_id = $2) \
             AND ($3::uuid IS NULL OR s.source_chat_id = $3) \
             AND ($4::text IS NULL OR s.created_by = $4) \
             ORDER BY s.created_at, s.id"
        );
        let rows: Vec<ScheduleRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(user.uuid())
            .bind(query.project.map(ProjectId::uuid))
            .bind(query.source_chat.map(ChatId::uuid))
            .bind(query.created_by.map(|by| by.as_str()))
            .fetch_all(self.pool())
            .await
            .map_err(failed("database.schedules.list.failed"))?;
        rows.into_iter().map(Schedule::try_from).collect()
    }

    async fn get(&self, user: UserId, id: ScheduleId) -> DomainResult<Option<Schedule>> {
        let sql = format!("SELECT {COLUMNS} FROM schedules s WHERE s.id = $1 AND s.user_id = $2");
        let row: Option<ScheduleRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(id.uuid())
            .bind(user.uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(failed("database.schedules.get.failed"))?;
        row.map(Schedule::try_from).transpose()
    }

    async fn insert(&self, schedule: &Schedule) -> DomainResult<()> {
        sqlx::query(
            "INSERT INTO schedules (created_at, updated_at, next_run_at, last_run_at, \
             consecutive_failures, id, user_id, project_id, model_id, source_chat_id, enabled, \
             name, cron, timezone, prompt, instructions, reasoning, on_ask, created_by, \
             last_status, last_error, disabled_reason) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, \
             $18, $19, $20, $21, $22)",
        )
        .bind(schedule.created_at())
        .bind(schedule.updated_at())
        .bind(schedule.next_run_at())
        .bind(schedule.last_run_at())
        .bind(count(schedule.consecutive_failures()))
        .bind(schedule.id().uuid())
        .bind(schedule.user_id().uuid())
        .bind(schedule.project_id().map(ProjectId::uuid))
        .bind(schedule.model_id().map(ModelId::uuid))
        .bind(schedule.source_chat_id().map(ChatId::uuid))
        .bind(schedule.enabled())
        .bind(schedule.name())
        .bind(schedule.rule().expression())
        .bind(schedule.rule().timezone())
        .bind(schedule.prompt())
        .bind(schedule.instructions())
        .bind(schedule.reasoning().map(ReasoningLevel::as_str))
        .bind(schedule.on_ask().as_str())
        .bind(schedule.created_by().as_str())
        .bind(schedule.last_status().map(|status| status.as_str()))
        .bind(schedule.last_error())
        .bind(schedule.disabled_reason())
        .execute(self.pool())
        .await
        .map_err(failed("database.schedules.insert.failed"))?;
        Ok(())
    }

    async fn update(&self, schedule: &Schedule) -> DomainResult<()> {
        sqlx::query(
            "UPDATE schedules SET updated_at = $3, model_id = $4, name = $5, prompt = $6, \
             instructions = $7, reasoning = $8, on_ask = $9 WHERE id = $1 AND user_id = $2",
        )
        .bind(schedule.id().uuid())
        .bind(schedule.user_id().uuid())
        .bind(schedule.updated_at())
        .bind(schedule.model_id().map(ModelId::uuid))
        .bind(schedule.name())
        .bind(schedule.prompt())
        .bind(schedule.instructions())
        .bind(schedule.reasoning().map(ReasoningLevel::as_str))
        .bind(schedule.on_ask().as_str())
        .execute(self.pool())
        .await
        .map_err(failed("database.schedules.update.failed"))?;
        Ok(())
    }

    async fn reschedule(&self, schedule: &Schedule, read: &Schedule) -> DomainResult<bool> {
        let updated = sqlx::query(
            "UPDATE schedules SET updated_at = $3, model_id = $4, name = $5, prompt = $6, \
             instructions = $7, reasoning = $8, on_ask = $9, cron = $10, timezone = $11, \
             enabled = $12, next_run_at = $13, consecutive_failures = $14, \
             disabled_reason = $15 \
             WHERE id = $1 AND user_id = $2 AND enabled = $16 \
             AND next_run_at IS NOT DISTINCT FROM $17 AND consecutive_failures = $18",
        )
        .bind(schedule.id().uuid())
        .bind(schedule.user_id().uuid())
        .bind(schedule.updated_at())
        .bind(schedule.model_id().map(ModelId::uuid))
        .bind(schedule.name())
        .bind(schedule.prompt())
        .bind(schedule.instructions())
        .bind(schedule.reasoning().map(ReasoningLevel::as_str))
        .bind(schedule.on_ask().as_str())
        .bind(schedule.rule().expression())
        .bind(schedule.rule().timezone())
        .bind(schedule.enabled())
        .bind(schedule.next_run_at())
        .bind(count(schedule.consecutive_failures()))
        .bind(schedule.disabled_reason())
        .bind(read.enabled())
        .bind(read.next_run_at())
        .bind(count(read.consecutive_failures()))
        .execute(self.pool())
        .await
        .map_err(failed("database.schedules.reschedule.failed"))?
        .rows_affected();
        Ok(updated > 0)
    }

    async fn delete(&self, user: UserId, id: ScheduleId) -> DomainResult<bool> {
        let deleted = sqlx::query("DELETE FROM schedules WHERE id = $1 AND user_id = $2")
            .bind(id.uuid())
            .bind(user.uuid())
            .execute(self.pool())
            .await
            .map_err(failed("database.schedules.delete.failed"))?
            .rows_affected();
        Ok(deleted > 0)
    }

    async fn count_enabled(&self, user: UserId) -> DomainResult<u32> {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM schedules WHERE user_id = $1 AND enabled")
                .bind(user.uuid())
                .fetch_one(self.pool())
                .await
                .map_err(failed("database.schedules.count.failed"))?;
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
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

    async fn due(
        &self,
        now: DateTime<Utc>,
        limit: u32,
        busy: &[UserId],
    ) -> DomainResult<Vec<(Schedule, Actor)>> {
        // The partial index on `next_run_at` (where `enabled`) finds the candidates; owners that
        // lib-rustauth disabled are skipped, and only the `administrator` role acts as an admin.
        // `turn` numbers each owner's due schedules, oldest first, so owners take turns; the
        // owners in `busy` come after everyone else.
        let sql = format!(
            "SELECT {COLUMNS}, s.is_admin FROM ( \
               SELECT s.*, u.role = 'administrator' AS is_admin, \
                 ROW_NUMBER() OVER (PARTITION BY s.user_id ORDER BY s.next_run_at, s.id) AS turn \
               FROM schedules s JOIN auth.users u ON u.id = s.user_id \
               WHERE s.enabled AND s.next_run_at <= $1 AND u.disabled_at IS NULL \
             ) s \
             ORDER BY s.user_id = ANY($3), s.turn, s.next_run_at, s.id LIMIT $2"
        );
        let busy: Vec<Uuid> = busy.iter().map(|user| user.uuid()).collect();
        let rows: Vec<DueRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(now)
            .bind(i64::from(limit))
            .bind(busy)
            .fetch_all(self.pool())
            .await
            .map_err(failed("database.schedules.due.failed"))?;
        rows.into_iter()
            .map(|row| {
                let schedule = Schedule::try_from(row.schedule)?;
                let actor = Actor::new(schedule.user_id(), row.is_admin);
                Ok((schedule, actor))
            })
            .collect()
    }

    async fn advance(
        &self,
        id: ScheduleId,
        expected: DateTime<Utc>,
        next: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> DomainResult<bool> {
        let claimed = sqlx::query(
            "UPDATE schedules SET next_run_at = $3, last_run_at = $4 \
             WHERE id = $1 AND enabled AND next_run_at = $2",
        )
        .bind(id.uuid())
        .bind(expected)
        .bind(next)
        .bind(now)
        .execute(self.pool())
        .await
        .map_err(failed("database.schedules.advance.failed"))?
        .rows_affected();
        Ok(claimed > 0)
    }

    async fn record(&self, id: ScheduleId, outcome: &RunRecord) -> DomainResult<()> {
        // `Schedule::recorded` is the reference. Every column reference is the row before the
        // update, so `failures` is spelled out where the disable rule needs it.
        const FAILURES: &str = "(CASE $3::text WHEN 'ok' THEN 0 \
             WHEN 'error' THEN consecutive_failures + 1 ELSE consecutive_failures END)";
        let sql = format!(
            "UPDATE schedules SET last_run_at = $2, last_status = $3, last_error = $4, \
             consecutive_failures = {FAILURES}, \
             enabled = enabled AND {FAILURES} < $5, \
             next_run_at = CASE WHEN enabled AND {FAILURES} >= $5 THEN NULL \
               ELSE next_run_at END, \
             disabled_reason = CASE WHEN enabled AND {FAILURES} >= $5 THEN $6 \
               ELSE disabled_reason END \
             WHERE id = $1"
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(id.uuid())
            .bind(outcome.started_at)
            .bind(outcome.status.as_str())
            .bind(outcome.error.as_deref())
            .bind(count(outcome.max_failures))
            .bind(&outcome.disable_reason)
            .execute(self.pool())
            .await
            .map_err(failed("database.schedules.record.failed"))?;
        Ok(())
    }
}
