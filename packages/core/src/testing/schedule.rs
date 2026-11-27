//! In-memory `ScheduleRepository`.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domains::chat::{Chat, ChatParts};
use crate::domains::schedule::{
    RunRecord, Schedule, ScheduleParts, ScheduleQuery, ScheduleRepository,
};
use crate::shared::actor::Actor;
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{ModelId, ScheduleId, UserId};
use crate::testing::store::{MemoryStore, Tables, duplicate};

#[async_trait]
impl ScheduleRepository for MemoryStore {
    async fn list(&self, user: UserId, query: ScheduleQuery) -> DomainResult<Vec<Schedule>> {
        let tables = self.tables();
        Ok(tables
            .schedules
            .values()
            .filter(|schedule| schedule.user_id() == user)
            .filter(|schedule| {
                query
                    .project
                    .is_none_or(|id| schedule.project_id() == Some(id))
            })
            .filter(|schedule| {
                query
                    .source_chat
                    .is_none_or(|id| schedule.source_chat_id() == Some(id))
            })
            .filter(|schedule| {
                query
                    .created_by
                    .is_none_or(|by| schedule.created_by() == by)
            })
            .cloned()
            .collect())
    }

    async fn get(&self, user: UserId, id: ScheduleId) -> DomainResult<Option<Schedule>> {
        let tables = self.tables();
        Ok(tables
            .schedules
            .get(&id)
            .filter(|schedule| schedule.user_id() == user)
            .cloned())
    }

    async fn insert(&self, schedule: &Schedule) -> DomainResult<()> {
        let mut tables = self.tables();
        if tables.schedules.contains_key(&schedule.id()) {
            return Err(duplicate("schedules"));
        }
        tables.check_schedule_references(schedule)?;
        tables.schedules.insert(schedule.id(), schedule.clone());
        Ok(())
    }

    async fn update(&self, schedule: &Schedule) -> DomainResult<()> {
        let mut tables = self.tables();
        tables.check_schedule_references(schedule)?;
        if let Some(stored) = tables.schedules.get_mut(&schedule.id())
            && stored.user_id() == schedule.user_id()
        {
            // The rule and the scheduler's columns belong to `reschedule`, `advance`, and
            // `record`.
            *stored = Schedule::restore(ScheduleParts {
                rule: stored.rule().clone(),
                enabled: stored.enabled(),
                next_run_at: stored.next_run_at(),
                consecutive_failures: stored.consecutive_failures(),
                disabled_reason: stored.disabled_reason().map(str::to_owned),
                last_run_at: stored.last_run_at(),
                last_status: stored.last_status(),
                last_error: stored.last_error().map(str::to_owned),
                ..parts(schedule)
            });
        }
        Ok(())
    }

    async fn reschedule(&self, schedule: &Schedule, read: &Schedule) -> DomainResult<bool> {
        let mut tables = self.tables();
        tables.check_schedule_references(schedule)?;
        let Some(stored) = tables.schedules.get_mut(&schedule.id()) else {
            return Ok(false);
        };
        let unchanged = stored.user_id() == schedule.user_id()
            && stored.enabled() == read.enabled()
            && stored.next_run_at() == read.next_run_at()
            && stored.consecutive_failures() == read.consecutive_failures();
        if unchanged {
            // The run columns belong to `advance` and `record`.
            *stored = Schedule::restore(ScheduleParts {
                last_run_at: stored.last_run_at(),
                last_status: stored.last_status(),
                last_error: stored.last_error().map(str::to_owned),
                ..parts(schedule)
            });
        }
        Ok(unchanged)
    }

    async fn delete(&self, user: UserId, id: ScheduleId) -> DomainResult<bool> {
        let mut tables = self.tables();
        let owned = tables
            .schedules
            .get(&id)
            .is_some_and(|schedule| schedule.user_id() == user);
        if owned {
            tables.delete_schedule(id);
        }
        Ok(owned)
    }

    async fn count_enabled(&self, user: UserId) -> DomainResult<u32> {
        let tables = self.tables();
        let count = tables
            .schedules
            .values()
            .filter(|schedule| schedule.user_id() == user && schedule.enabled())
            .count();
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    async fn disable_for_model(
        &self,
        user: UserId,
        model: ModelId,
        reason: &str,
    ) -> DomainResult<u64> {
        let mut tables = self.tables();
        let mut count = 0;
        for schedule in tables.schedules.values_mut() {
            if schedule.user_id() == user
                && schedule.enabled()
                && schedule.model_id() == Some(model)
            {
                *schedule = schedule.clone().disabled(Some(reason.to_owned()));
                count += 1;
            }
        }
        Ok(count)
    }

    async fn due(
        &self,
        now: DateTime<Utc>,
        limit: u32,
        busy: &[UserId],
    ) -> DomainResult<Vec<(Schedule, Actor)>> {
        let tables = self.tables();
        let mut due: Vec<(Schedule, Actor)> = tables
            .schedules
            .values()
            .filter(|schedule| schedule.enabled())
            .filter(|schedule| schedule.next_run_at().is_some_and(|next| next <= now))
            .filter_map(|schedule| {
                let owner = tables
                    .users
                    .get(&schedule.user_id())
                    .copied()
                    .unwrap_or_default();
                (!owner.disabled).then(|| {
                    (
                        schedule.clone(),
                        Actor::new(schedule.user_id(), owner.is_admin),
                    )
                })
            })
            .collect();
        due.sort_by_key(|(schedule, _)| (schedule.next_run_at(), schedule.id()));
        // Turns: each owner's oldest first, then each owner's second, and so on; busy owners
        // after everyone else.
        let mut turns: HashMap<UserId, usize> = HashMap::new();
        let mut ranked: Vec<(bool, usize, (Schedule, Actor))> = due
            .into_iter()
            .map(|entry| {
                let owner = entry.0.user_id();
                let turn = turns.entry(owner).or_default();
                *turn += 1;
                (busy.contains(&owner), *turn, entry)
            })
            .collect();
        ranked.sort_by_key(|(busy, turn, (schedule, _))| {
            (*busy, *turn, schedule.next_run_at(), schedule.id())
        });
        ranked.truncate(limit as usize);
        Ok(ranked.into_iter().map(|(_, _, entry)| entry).collect())
    }

    async fn advance(
        &self,
        id: ScheduleId,
        expected: DateTime<Utc>,
        next: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> DomainResult<bool> {
        let mut tables = self.tables();
        match tables.schedules.get_mut(&id) {
            Some(schedule) if schedule.enabled() && schedule.next_run_at() == Some(expected) => {
                *schedule = schedule.clone().advanced(next, now);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn record(&self, id: ScheduleId, outcome: &RunRecord) -> DomainResult<()> {
        let mut tables = self.tables();
        if let Some(schedule) = tables.schedules.get_mut(&id) {
            *schedule = schedule.clone().recorded(outcome);
        }
        Ok(())
    }
}

impl Tables {
    /// The composite foreign keys of `project_id` and `model_id`.
    fn check_schedule_references(&self, schedule: &Schedule) -> DomainResult<()> {
        let user = schedule.user_id();
        let project_ok = schedule.project_id().is_none_or(|project| {
            self.projects
                .get(&project)
                .is_some_and(|stored| stored.user_id() == user)
        });
        let model_ok = schedule.model_id().is_none_or(|model| {
            self.models
                .get(&model)
                .is_some_and(|stored| stored.user_id() == user)
        });
        if project_ok && model_ok {
            Ok(())
        } else {
            Err(DomainError::Internal {
                key: "testing.schedules.foreign-key".to_owned(),
                source: None,
            })
        }
    }

    /// Deletes a schedule; the chats of its runs lose `schedule_id`.
    pub(crate) fn delete_schedule(&mut self, id: ScheduleId) {
        self.schedules.remove(&id);
        for chat in self.chats.values_mut() {
            if chat.schedule_id() == Some(id) {
                *chat = Chat::restore(ChatParts {
                    schedule_id: None,
                    ..chat.to_parts()
                });
            }
        }
    }
}

pub(crate) fn parts(schedule: &Schedule) -> ScheduleParts {
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
