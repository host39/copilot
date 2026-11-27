//! Running prepared turns: [`launch`] starts a turn in its reserved slot on a task of its own,
//! for the chat routes and the scheduler alike, and [`follow_up`] records how a scheduled run's
//! chat ended after the run paused for approval.

use std::sync::Arc;

use copilot_contracts::UserEvent;
use copilot_core::{Chat, PreparedTurn, RUN_FAILED, RunMode, RunOutcome, TurnOutcome, TurnSink};
use tokio::task::JoinHandle;

use crate::config::AppState;
use crate::helpers::runs::Reservation;

/// Starts the prepared turn in its reserved slot ([`Reservation::start`]: streams may attach
/// from now on) and runs it on a task of its own, and returns that task: its output is how the
/// run ended, `None` when it failed with an error or panicked (dropping the reservation then
/// ends its streams with `error` and `done`). Nothing cancels the run but
/// [`RunRegistry::stop`] and shutdown; the slot is released when the run ends.
///
/// [`RunRegistry::stop`]: crate::helpers::runs::RunRegistry::stop
pub fn launch(
    state: &AppState,
    reservation: Reservation,
    turn: PreparedTurn,
) -> JoinHandle<Option<TurnOutcome>> {
    let engine = Arc::clone(&state.engine);
    let chat = turn.chat.id();
    // A turn the owner runs in a scheduled run's chat (after it paused for approval) records how
    // that run ended; the scheduler records the run's own turn.
    let follow = (turn.mode == RunMode::Interactive && turn.chat.schedule_id().is_some())
        .then(|| (state.clone(), turn.chat.clone()));
    reservation.serve(turn.provider.id());
    reservation.start();
    tokio::spawn(async move {
        let task = tokio::spawn({
            let sink: Arc<dyn TurnSink> = reservation.sink();
            let cancel = reservation.cancel();
            async move { engine.run(turn, sink, cancel).await }
        });
        let outcome = match task.await {
            Ok(Ok(outcome)) => {
                tracing::info!(chat_id = %chat, status = %outcome.status, "api.chats.run.finished");
                Some(outcome)
            }
            Ok(Err(error)) => {
                tracing::error!(chat_id = %chat, error = %error, "api.chats.run.failed");
                None
            }
            Err(error) => {
                tracing::error!(chat_id = %chat, error = %error, "api.chats.run.panicked");
                None
            }
        };
        // A run that panicked never sent `done`: dropping the reservation ends its streams.
        drop(reservation);
        if let Some((state, chat)) = follow {
            let ended = outcome.as_ref().map_or_else(
                || RunOutcome::Failed(RUN_FAILED.to_owned()),
                RunOutcome::from,
            );
            follow_up(&state, &chat, ended).await;
        }
        outcome
    })
}

/// Records how a scheduled run's chat ended after the run paused (see
/// `ScheduleService::follow_up`), and tells the owner's event streams.
pub async fn follow_up(state: &AppState, chat: &Chat, outcome: RunOutcome) {
    let Some(schedule) = chat.schedule_id() else {
        return;
    };
    match state.schedules.follow_up(chat, outcome).await {
        Ok(()) => state.runs.publish(
            chat.user_id(),
            UserEvent::ScheduleUpdated {
                schedule_id: schedule.uuid(),
            },
        ),
        Err(error) => {
            tracing::warn!(schedule_id = %schedule, error = %error, "api.chats.follow-up.failed");
        }
    }
}
