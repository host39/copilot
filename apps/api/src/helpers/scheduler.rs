//! The scheduler loop (`docs/design/copilot.md`, Schedules). The system clock decides when a run
//! happens; the model only writes schedule rows (the built-in schedule tools).
//!
//! Every `SCHEDULER_TICK_SECS` (and once at start, so fires missed while the server was down
//! run at once), a tick takes the free scheduled-run slots (`MAX_SCHEDULED_RUNS` in all, shared
//! with Run now) and claims at most that many due schedules with `ScheduleService::fire_due`: a
//! compare-and-set of each schedule's next run, so of two schedulers on one database only one
//! claims a fire, and missed fires collapse into one run. Owners take turns, and owners who hold
//! a slot already ([`RunRegistry::holders`]) come after everyone else, so one user's schedules
//! never keep another's waiting while a slot is free. Each claim starts a run as the schedule's
//! owner ([`fire`]) through the same services as a chat: the registry reserves the run's new
//! chat, `ChatService::prepare_scheduled` writes it with the prompt, and the engine answers
//! unattended (no schedule tools; `ask` tools follow the schedule's `on_ask`). A run still live
//! after `SCHEDULED_RUN_MAX_SECS` is stopped and recorded as failed ([`RUN_TIMED_OUT`]), so no
//! run holds its slot for long.
//!
//! [`RunRegistry::holders`]: crate::helpers::runs::RunRegistry::holders
//!
//! How the run ended is recorded in core's terms (`RunOutcome`): an answer resets the failure
//! count, a failure raises it and the third in a row turns the schedule off, an approval keeps
//! it, and a schedule whose model was removed or turned off pauses at once. The owner's event
//! streams hear `schedule.updated` after each claim and each record, and the run's own
//! `run.started` and `run.finished`.

use std::time::Duration;

use chrono::{DateTime, Utc};
use copilot_contracts::UserEvent;
use copilot_core::shared::constants::{
    MAX_SCHEDULED_RUNS, SCHEDULED_RUN_MAX_SECS, SCHEDULER_TICK_SECS, SHUTDOWN_DRAIN_SECS,
};
use copilot_core::{
    ChatId, DomainError, DomainResult, Firing, MODEL_UNAVAILABLE, RUN_FAILED, RUN_TIMED_OUT,
    RunOutcome, ScheduleId, ScheduledTurn, TurnOutcome, UserId,
};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use crate::config::AppState;
use crate::helpers::runs::{Reservation, Slot};
use crate::routes::chats;

/// Claims due schedules and starts their runs. Cheap to clone.
#[derive(Clone)]
pub struct Scheduler {
    state: AppState,
}

impl Scheduler {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Ticks until `shutdown` is cancelled: right away, then every `SCHEDULER_TICK_SECS`. A tick
    /// that takes longer delays the next one instead of bunching them up.
    pub fn start(self, shutdown: CancellationToken) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(SCHEDULER_TICK_SECS));
            interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    _ = interval.tick() => {
                        self.tick(Utc::now()).await;
                    }
                }
            }
            tracing::info!("api.scheduler.stopped");
        })
    }

    /// One tick at `now`: claims at most as many due schedules as there are free scheduled-run
    /// slots and starts their runs. Returns the chats of the runs it started. Does nothing once
    /// the server is shutting down.
    pub async fn tick(&self, now: DateTime<Utc>) -> Vec<ChatId> {
        let mut started = Vec::new();
        for (slot, firing) in self.claim(now).await {
            let schedule = firing.schedule.id();
            tracing::info!(schedule_id = %schedule, chat_id = %firing.chat_id, "api.scheduler.fired");
            // The claim moved the next run.
            let owner = firing.actor.user_id;
            updated(&self.state, owner, schedule);
            let reservation = match self
                .state
                .runs
                .reserve_scheduled(slot, owner, firing.chat_id)
            {
                Ok(reservation) => reservation,
                Err(error) => {
                    tracing::warn!(schedule_id = %schedule, error = %error, "api.scheduler.run.skipped");
                    continue;
                }
            };
            if let Ok(chat) = fire(&self.state, reservation, firing).await {
                started.push(chat);
            }
        }
        started
    }

    /// The free scheduled-run slots, each with a due schedule claimed for it (none while the
    /// server shuts down). Slots left over are freed.
    async fn claim(&self, now: DateTime<Utc>) -> Vec<(Slot, Firing)> {
        if self.state.runs.is_closing() {
            return Vec::new();
        }
        let slots: Vec<Slot> = std::iter::from_fn(|| self.state.runs.slot())
            .take(MAX_SCHEDULED_RUNS)
            .collect();
        if slots.is_empty() {
            return Vec::new();
        }
        let limit = u32::try_from(slots.len()).unwrap_or(u32::MAX);
        let busy = self.state.runs.holders();
        match self.state.schedules.fire_due(now, limit, &busy).await {
            // `fire_due` claims at most `limit`, so every claim gets a slot.
            Ok(firings) => slots.into_iter().zip(firings).collect(),
            Err(error) => {
                tracing::error!(error = %error, "api.scheduler.tick.failed");
                Vec::new()
            }
        }
    }
}

/// Starts the run of `firing` (a claimed fire or a Run now) in its `reservation` (its new chat,
/// reserved in a scheduled-run slot), as the schedule's owner: writes the chat and the prompt,
/// and runs the turn unattended on a task of its own, which stops it after
/// `SCHEDULED_RUN_MAX_SECS` and records how it ended. Returns the run's chat.
///
/// A run that cannot start is recorded too and returns its error: a schedule whose model was
/// removed or turned off pauses (`MODEL_UNAVAILABLE`, with the reason in plain words), and any
/// other failure counts as a failed run. While the server shuts down nothing is recorded
/// (`SERVER_STOPPING`).
pub async fn fire(
    state: &AppState,
    reservation: Reservation,
    firing: Firing,
) -> DomainResult<ChatId> {
    let schedule = firing.schedule.id();
    let owner = firing.actor.user_id;
    let at = firing.at;
    let chat = firing.chat_id;
    let turn = match state.chats.prepare_scheduled(firing).await {
        Ok(ScheduledTurn::Ready(turn)) => *turn,
        Ok(ScheduledTurn::Paused { reason, message }) => {
            tracing::info!(schedule_id = %schedule, reason, "api.scheduler.paused");
            let outcome = RunOutcome::Paused {
                reason: reason.to_owned(),
                message: message.to_owned(),
            };
            record(state, schedule, owner, outcome, at).await;
            return Err(DomainError::business_rule(message, MODEL_UNAVAILABLE));
        }
        Err(error) => {
            tracing::warn!(schedule_id = %schedule, error = %error, "api.scheduler.run.not-started");
            record(state, schedule, owner, RunOutcome::not_started(&error), at).await;
            return Err(error);
        }
    };
    let cancel = reservation.cancel();
    let task = chats::launch(state, reservation, turn);
    let state = state.clone();
    state.tasks.clone().spawn(async move {
        let limit = Duration::from_secs(SCHEDULED_RUN_MAX_SECS);
        let outcome = ended(task, &cancel, limit).await;
        record(&state, schedule, owner, outcome, at).await;
    });
    Ok(chat)
}

/// How the run of `task` ended, as the schedule records it. A run still live after `limit` is
/// cancelled (it saves its partial answer) and fails with [`RUN_TIMED_OUT`].
async fn ended(
    mut task: JoinHandle<Option<TurnOutcome>>,
    cancel: &CancellationToken,
    limit: Duration,
) -> RunOutcome {
    match tokio::time::timeout(limit, &mut task).await {
        Ok(Ok(Some(outcome))) => RunOutcome::from(&outcome),
        Ok(Ok(None) | Err(_)) => RunOutcome::Failed(RUN_FAILED.to_owned()),
        Err(_) => {
            tracing::warn!(limit_secs = limit.as_secs(), "api.scheduler.run.timed-out");
            cancel.cancel();
            // Waits for the cancelled answer to be saved, so the record comes after it.
            let _ = task.await;
            RunOutcome::Failed(RUN_TIMED_OUT.to_owned())
        }
    }
}

/// Stores how the run of `schedule` that started at `at` ended, then tells the owner's event
/// streams.
async fn record(
    state: &AppState,
    schedule: ScheduleId,
    owner: UserId,
    outcome: RunOutcome,
    at: DateTime<Utc>,
) {
    if let Err(error) = state.schedules.record(schedule, outcome, at).await {
        tracing::error!(schedule_id = %schedule, error = %error, "api.scheduler.record.failed");
    }
    updated(state, owner, schedule);
}

/// Tells the owner's event streams that `schedule` changed.
fn updated(state: &AppState, owner: UserId, schedule: ScheduleId) {
    state.runs.publish(
        owner,
        UserEvent::ScheduleUpdated {
            schedule_id: schedule.uuid(),
        },
    );
}

/// Waits up to `SHUTDOWN_DRAIN_SECS` for the scheduled runs to record how they ended; call it
/// after the runs drained and before the pool closes. `false` when a record was still pending.
pub async fn settle(state: &AppState) -> bool {
    state.tasks.close();
    tokio::time::timeout(Duration::from_secs(SHUTDOWN_DRAIN_SECS), state.tasks.wait())
        .await
        .is_ok()
}

#[cfg(test)]
mod tests;
