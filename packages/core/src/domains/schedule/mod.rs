//! `schedule`: prompts that run on a cron rule in a time zone (`CronRule`, the minimum
//! interval, previews, compare-and-set firing, failure counting) and the built-in schedule tools.

mod builtin;
mod cron;
mod entities;
mod ports;
mod service;

pub use crate::domains::schedule::builtin::{
    SchedulePrompt, ScheduleTool, ToolOrigin, cancel_target,
};
pub use crate::domains::schedule::cron::CronRule;
pub use crate::domains::schedule::entities::{
    CreatedBy, MODEL_REMOVED, MODEL_TURNED_OFF, OnAsk, RunOutcome, RunRecord, RunStatus, Schedule,
    ScheduleParts,
};
pub use crate::domains::schedule::ports::{ScheduleQuery, ScheduleRepository};
pub use crate::domains::schedule::service::{
    Firing, NewSchedule, SchedulePreview, ScheduleService, ScheduleUpdate,
};
