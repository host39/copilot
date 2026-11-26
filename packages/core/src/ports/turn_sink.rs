//! Live turn events port.
//!
//! `apps/api` implements [`TurnSink`] in its run registry: it numbers each event (`seq`), keeps
//! it in the run's replay buffer, and broadcasts it to attached SSE streams. The port is frozen
//! from P1.3 on.

use crate::domains::conversation::TurnEvent;

/// Receives the events of a run, in order.
pub trait TurnSink: Send + Sync {
    /// Must not block: the engine calls it between provider events.
    fn emit(&self, event: TurnEvent);
}
