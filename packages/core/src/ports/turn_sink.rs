//! Live turn events port.
//!
//! `apps/api` implements [`TurnSink`] in its run registry: it numbers each event (`seq`), keeps
//! it in the run's replay buffer, and broadcasts it to attached SSE streams. The port is frozen
//! from P1.3 on; P3.1 added [`TurnSink::ended`] (with a default, so other sinks need nothing).

use crate::domains::conversation::{TurnEvent, TurnOutcome};

/// Receives the events of a run, in order.
pub trait TurnSink: Send + Sync {
    /// Must not block: the engine calls it between provider events.
    fn emit(&self, event: TurnEvent);

    /// How the run ended, just before its last event (`done`). A run that failed with an error
    /// (an `error` event, then `done`) has no outcome and never calls it. The run registry
    /// publishes the owner's `approval.pending` and `run.finished` from it.
    fn ended(&self, _outcome: &TurnOutcome) {}
}
