//! A `TurnSink` that keeps every event.

use std::sync::Mutex;

use crate::domains::conversation::TurnEvent;
use crate::ports::turn_sink::TurnSink;

#[derive(Debug, Default)]
pub struct RecordingSink(Mutex<Vec<TurnEvent>>);

impl RecordingSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every event so far, in order.
    pub fn events(&self) -> Vec<TurnEvent> {
        self.0.lock().expect("sink lock").clone()
    }
}

impl TurnSink for RecordingSink {
    fn emit(&self, event: TurnEvent) {
        self.0.lock().expect("sink lock").push(event);
    }
}
