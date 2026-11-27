//! A `TurnSink` that keeps every event and outcome.

use std::sync::Mutex;

use crate::domains::conversation::{TurnEvent, TurnOutcome};
use crate::ports::turn_sink::TurnSink;

#[derive(Debug, Default)]
pub struct RecordingSink {
    events: Mutex<Vec<TurnEvent>>,
    outcomes: Mutex<Vec<(TurnOutcome, usize)>>,
}

impl RecordingSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every event so far, in order.
    pub fn events(&self) -> Vec<TurnEvent> {
        self.events.lock().expect("sink lock").clone()
    }

    /// Every outcome reported so far, each with the number of events sent before it.
    pub fn outcomes(&self) -> Vec<(TurnOutcome, usize)> {
        self.outcomes.lock().expect("sink lock").clone()
    }
}

impl TurnSink for RecordingSink {
    fn emit(&self, event: TurnEvent) {
        self.events.lock().expect("sink lock").push(event);
    }

    fn ended(&self, outcome: &TurnOutcome) {
        let sent = self.events.lock().expect("sink lock").len();
        self.outcomes
            .lock()
            .expect("sink lock")
            .push((outcome.clone(), sent));
    }
}
