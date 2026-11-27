//! Schedules: prompts that run on a cron rule, each firing in a new chat.

use chrono::{DateTime, Utc};

use crate::domains::schedule::cron::CronRule;
use crate::shared::constants::{NAME_MAX_CHARS, SCHEDULE_MAX_FAILURES};
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{ChatId, ModelId, ProjectId, ScheduleId, UserId};
use crate::shared::reasoning::ReasoningLevel;
use crate::shared::text;
use crate::shared::text::text_enum;

/// Why a schedule paused after its model went away (`disabled_reason`).
pub const MODEL_REMOVED: &str = "Model removed";
/// Why a schedule paused after its model was turned off.
pub const MODEL_TURNED_OFF: &str = "Model turned off";

/// What an unattended run does with a tool call that would ask (`on_ask`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum OnAsk {
    /// "Deny and continue": the call gets a denied result and the run goes on.
    #[default]
    Deny,
    /// "Pause until I approve": the run stops awaiting approval.
    Pause,
}

/// Who created a schedule. The UI marks the assistant's, and the built-in schedule tools see
/// only those.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CreatedBy {
    User,
    Assistant,
}

/// How the last run ended (`last_status`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RunStatus {
    Ok,
    Error,
    AwaitingApproval,
}

text_enum!(OnAsk { Deny => "deny", Pause => "pause" });
text_enum!(CreatedBy { User => "user", Assistant => "assistant" });
text_enum!(RunStatus { Ok => "ok", Error => "error", AwaitingApproval => "awaiting_approval" });

/// The failure a run records when it could not start for an internal reason (the log has it).
pub const START_FAILED: &str = "The run could not start. It will try again at the next run.";
/// The failure a run records when it stopped with an internal error (the log has it).
pub const RUN_FAILED: &str = "Something went wrong while answering.";
/// The failure a run records when it ran past `SCHEDULED_RUN_MAX_SECS` and was stopped.
pub const RUN_TIMED_OUT: &str = "The run took longer than 30 minutes and was stopped.";

/// How a run ended, as the scheduler reports it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunOutcome {
    Ok,
    /// A human-readable reason.
    Failed(String),
    AwaitingApproval,
    /// The run could not start because the schedule's model is gone or turned off: the schedule
    /// pauses at once with `reason` ([`MODEL_REMOVED`] or [`MODEL_TURNED_OFF`]), and `message`
    /// says why in a sentence.
    Paused {
        reason: String,
        message: String,
    },
}

impl RunOutcome {
    /// How a run whose start failed with `error` ends: a failed run with the error's message
    /// (internal errors read as [`START_FAILED`]).
    pub fn not_started(error: &DomainError) -> Self {
        match error {
            DomainError::Internal { .. } => Self::Failed(START_FAILED.to_owned()),
            other => Self::Failed(other.to_string()),
        }
    }
}

/// What `ScheduleRepository::record` writes, built by core so the failure rule stays here:
/// `last_run_at`, `last_status`, `last_error`, `consecutive_failures` (reset by `ok`, raised
/// by `error`, kept by `awaiting_approval`), and, when the failures reach `max_failures` on an
/// enabled schedule, `enabled = false`, `next_run_at = NULL`, and `disabled_reason`. A paused
/// run is an `error` with `max_failures` 0, so it disables the schedule at once.
/// [`Schedule::recorded`] is the reference implementation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunRecord {
    pub status: RunStatus,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub max_failures: u32,
    pub disable_reason: String,
}

impl RunRecord {
    pub fn new(outcome: RunOutcome, started_at: DateTime<Utc>) -> Self {
        let mut max_failures = SCHEDULE_MAX_FAILURES;
        let mut disable_reason = format!("Failed {SCHEDULE_MAX_FAILURES} times in a row");
        let (status, error) = match outcome {
            RunOutcome::Ok => (RunStatus::Ok, None),
            RunOutcome::Failed(reason) => (RunStatus::Error, Some(reason)),
            RunOutcome::AwaitingApproval => (RunStatus::AwaitingApproval, None),
            RunOutcome::Paused { reason, message } => {
                max_failures = 0;
                disable_reason = reason;
                (RunStatus::Error, Some(message))
            }
        };
        Self {
            status,
            error,
            started_at,
            max_failures,
            disable_reason,
        }
    }
}

/// A schedule (`schedules`). In a project it runs with the project's instructions, files, tools,
/// and default reasoning plus its own model; standalone it runs with its own instructions, no
/// files, and no tools. `next_run_at` is `None` while disabled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Schedule {
    id: ScheduleId,
    user_id: UserId,
    project_id: Option<ProjectId>,
    model_id: Option<ModelId>,
    source_chat_id: Option<ChatId>,
    name: String,
    rule: CronRule,
    prompt: String,
    instructions: String,
    reasoning: Option<ReasoningLevel>,
    on_ask: OnAsk,
    created_by: CreatedBy,
    enabled: bool,
    next_run_at: Option<DateTime<Utc>>,
    last_run_at: Option<DateTime<Utc>>,
    last_status: Option<RunStatus>,
    last_error: Option<String>,
    consecutive_failures: u32,
    disabled_reason: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Every field of a stored schedule, for [`Schedule::restore`].
#[derive(Clone, Debug)]
pub struct ScheduleParts {
    pub id: ScheduleId,
    pub user_id: UserId,
    pub project_id: Option<ProjectId>,
    pub model_id: Option<ModelId>,
    pub source_chat_id: Option<ChatId>,
    pub name: String,
    pub rule: CronRule,
    pub prompt: String,
    pub instructions: String,
    pub reasoning: Option<ReasoningLevel>,
    pub on_ask: OnAsk,
    pub created_by: CreatedBy,
    pub enabled: bool,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_status: Option<RunStatus>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    pub disabled_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Schedule {
    /// A new, enabled, standalone schedule by the user, due at the rule's next fire after `now`.
    pub fn create(
        user_id: UserId,
        name: &str,
        rule: CronRule,
        prompt: &str,
        now: DateTime<Utc>,
    ) -> DomainResult<Self> {
        Ok(Self {
            id: ScheduleId::generate(),
            user_id,
            project_id: None,
            model_id: None,
            source_chat_id: None,
            name: schedule_name(name)?,
            next_run_at: rule.next_after(now),
            rule,
            prompt: schedule_prompt(prompt)?,
            instructions: String::new(),
            reasoning: None,
            on_ask: OnAsk::Deny,
            created_by: CreatedBy::User,
            enabled: true,
            last_run_at: None,
            last_status: None,
            last_error: None,
            consecutive_failures: 0,
            disabled_reason: None,
            created_at: now,
            updated_at: now,
        })
    }

    /// A row read back from storage.
    pub fn restore(parts: ScheduleParts) -> Self {
        Self {
            id: parts.id,
            user_id: parts.user_id,
            project_id: parts.project_id,
            model_id: parts.model_id,
            source_chat_id: parts.source_chat_id,
            name: parts.name,
            rule: parts.rule,
            prompt: parts.prompt,
            instructions: parts.instructions,
            reasoning: parts.reasoning,
            on_ask: parts.on_ask,
            created_by: parts.created_by,
            enabled: parts.enabled,
            next_run_at: parts.next_run_at,
            last_run_at: parts.last_run_at,
            last_status: parts.last_status,
            last_error: parts.last_error,
            consecutive_failures: parts.consecutive_failures,
            disabled_reason: parts.disabled_reason,
            created_at: parts.created_at,
            updated_at: parts.updated_at,
        }
    }

    pub fn with_name(mut self, name: &str) -> DomainResult<Self> {
        self.name = schedule_name(name)?;
        Ok(self)
    }

    pub fn with_prompt(mut self, prompt: &str) -> DomainResult<Self> {
        self.prompt = schedule_prompt(prompt)?;
        Ok(self)
    }

    /// A new rule; an enabled schedule is due at its next fire after `now`.
    #[must_use]
    pub fn with_rule(mut self, rule: CronRule, now: DateTime<Utc>) -> Self {
        if self.enabled {
            self.next_run_at = rule.next_after(now);
        }
        self.rule = rule;
        self
    }

    /// Runs in `project` (with the project's instructions), or standalone. A project schedule
    /// keeps no instructions of its own.
    #[must_use]
    pub fn in_project(mut self, project: Option<ProjectId>) -> Self {
        self.project_id = project;
        if project.is_some() {
            self.instructions.clear();
        }
        self
    }

    /// Instructions of a standalone schedule (trimmed); ignored in a project.
    #[must_use]
    pub fn with_instructions(mut self, instructions: &str) -> Self {
        if self.project_id.is_none() {
            self.instructions = text::trimmed(instructions);
        }
        self
    }

    /// Check that the model belongs to the owner before calling this.
    #[must_use]
    pub fn with_model(mut self, model: Option<ModelId>) -> Self {
        self.model_id = model;
        self
    }

    /// `None` uses the project's level, then the model default.
    #[must_use]
    pub fn with_reasoning(mut self, reasoning: Option<ReasoningLevel>) -> Self {
        self.reasoning = reasoning;
        self
    }

    #[must_use]
    pub fn with_on_ask(mut self, on_ask: OnAsk) -> Self {
        self.on_ask = on_ask;
        self
    }

    /// Created by the assistant through `schedule_prompt` in `chat`.
    #[must_use]
    pub fn by_assistant(mut self, chat: ChatId) -> Self {
        self.created_by = CreatedBy::Assistant;
        self.source_chat_id = Some(chat);
        self
    }

    /// Turns the schedule on: due at the next fire after `now`, with the failure count and the
    /// pause reason cleared.
    #[must_use]
    pub fn enabled_at(mut self, now: DateTime<Utc>) -> Self {
        self.enabled = true;
        self.next_run_at = self.rule.next_after(now);
        self.consecutive_failures = 0;
        self.disabled_reason = None;
        self
    }

    /// Turns the schedule off; `reason` is shown in red (`None` when the user turned it off).
    #[must_use]
    pub fn disabled(mut self, reason: Option<String>) -> Self {
        self.enabled = false;
        self.next_run_at = None;
        self.disabled_reason = reason;
        self
    }

    /// What `ScheduleRepository::advance` writes after a successful claim.
    #[must_use]
    pub fn advanced(mut self, next: DateTime<Utc>, now: DateTime<Utc>) -> Self {
        self.next_run_at = Some(next);
        self.last_run_at = Some(now);
        self
    }

    /// What `ScheduleRepository::record` writes (see [`RunRecord`]).
    #[must_use]
    pub fn recorded(mut self, record: &RunRecord) -> Self {
        self.last_run_at = Some(record.started_at);
        self.last_status = Some(record.status);
        self.last_error.clone_from(&record.error);
        match record.status {
            RunStatus::Ok => self.consecutive_failures = 0,
            RunStatus::Error => self.consecutive_failures += 1,
            RunStatus::AwaitingApproval => {}
        }
        if self.enabled && self.consecutive_failures >= record.max_failures {
            self = self.disabled(Some(record.disable_reason.clone()));
        }
        self
    }

    #[must_use]
    pub fn touched(mut self, now: DateTime<Utc>) -> Self {
        self.updated_at = now;
        self
    }

    pub fn id(&self) -> ScheduleId {
        self.id
    }

    pub fn user_id(&self) -> UserId {
        self.user_id
    }

    /// `None` for a standalone schedule.
    pub fn project_id(&self) -> Option<ProjectId> {
        self.project_id
    }

    /// `None` after the model was deleted.
    pub fn model_id(&self) -> Option<ModelId> {
        self.model_id
    }

    pub fn source_chat_id(&self) -> Option<ChatId> {
        self.source_chat_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn rule(&self) -> &CronRule {
        &self.rule
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    pub fn reasoning(&self) -> Option<ReasoningLevel> {
        self.reasoning
    }

    pub fn on_ask(&self) -> OnAsk {
        self.on_ask
    }

    pub fn created_by(&self) -> CreatedBy {
        self.created_by
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn next_run_at(&self) -> Option<DateTime<Utc>> {
        self.next_run_at
    }

    pub fn last_run_at(&self) -> Option<DateTime<Utc>> {
        self.last_run_at
    }

    pub fn last_status(&self) -> Option<RunStatus> {
        self.last_status
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    pub fn disabled_reason(&self) -> Option<&str> {
        self.disabled_reason.as_deref()
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

fn schedule_name(name: &str) -> DomainResult<String> {
    text::name(name, "Schedule name", "name", NAME_MAX_CHARS)
}

fn schedule_prompt(prompt: &str) -> DomainResult<String> {
    let prompt = text::trimmed(prompt);
    if prompt.is_empty() {
        return Err(DomainError::validation("Enter the prompt to run.").with_field("prompt"));
    }
    Ok(prompt)
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};
    use uuid::Uuid;

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    fn schedule() -> Schedule {
        let rule = CronRule::parse("0 9 * * *", "UTC", now()).unwrap();
        Schedule::create(
            UserId::new(Uuid::now_v7()),
            "Digest",
            rule,
            "Summarize the news.",
            now(),
        )
        .unwrap()
    }

    fn failed(at: DateTime<Utc>) -> RunRecord {
        RunRecord::new(RunOutcome::Failed("Key rejected.".to_owned()), at)
    }

    #[test]
    fn starts_enabled_and_due_at_the_next_fire() {
        let schedule = schedule();
        assert!(schedule.enabled());
        assert_eq!(schedule.created_by(), CreatedBy::User);
        assert_eq!(schedule.on_ask(), OnAsk::Deny);
        assert_eq!(schedule.next_run_at(), Some(now() + Duration::hours(9)));
        assert_eq!(schedule.consecutive_failures(), 0);
    }

    #[test]
    fn requires_a_name_and_a_prompt() {
        let rule = CronRule::parse("0 9 * * *", "UTC", now()).unwrap();
        let user = UserId::new(Uuid::now_v7());
        let error = Schedule::create(user, " ", rule.clone(), "Hi", now()).unwrap_err();
        assert_eq!(error.to_string(), "Schedule name is required.");
        let error = Schedule::create(user, "Digest", rule, " \n ", now()).unwrap_err();
        assert_eq!(error.to_string(), "Enter the prompt to run.");
    }

    #[test]
    fn keeps_instructions_only_without_a_project() {
        let standalone = schedule().with_instructions(" Be brief. ");
        assert_eq!(standalone.instructions(), "Be brief.");
        let project = standalone.in_project(Some(ProjectId::generate()));
        assert_eq!(project.instructions(), "");
        assert_eq!(project.with_instructions("Ignored").instructions(), "");
    }

    #[test]
    fn disabling_clears_the_next_run_and_enabling_resets_failures() {
        let schedule = schedule().disabled(Some(MODEL_REMOVED.to_owned()));
        assert!(!schedule.enabled());
        assert_eq!(schedule.next_run_at(), None);
        assert_eq!(schedule.disabled_reason(), Some("Model removed"));
        let later = now() + Duration::days(2);
        let schedule = schedule.recorded(&failed(later)).enabled_at(later);
        assert!(schedule.enabled());
        assert_eq!(schedule.next_run_at(), Some(later + Duration::hours(9)));
        assert_eq!(schedule.disabled_reason(), None);
        assert_eq!(schedule.consecutive_failures(), 0);
    }

    #[test]
    fn a_new_rule_moves_the_next_run_only_while_enabled() {
        let every_ten = CronRule::parse("0 10 * * *", "UTC", now()).unwrap();
        let schedule = schedule().with_rule(every_ten.clone(), now());
        assert_eq!(schedule.next_run_at(), Some(now() + Duration::hours(10)));
        let paused = schedule.disabled(None).with_rule(every_ten, now());
        assert_eq!(paused.next_run_at(), None);
    }

    #[test]
    fn three_failures_in_a_row_disable_the_schedule() {
        let mut schedule = schedule();
        for count in 1..=2 {
            schedule = schedule.recorded(&failed(now()));
            assert_eq!(schedule.consecutive_failures(), count);
            assert!(schedule.enabled());
        }
        assert_eq!(schedule.last_status(), Some(RunStatus::Error));
        assert_eq!(schedule.last_error(), Some("Key rejected."));
        let schedule = schedule.recorded(&failed(now()));
        assert!(!schedule.enabled());
        assert_eq!(schedule.next_run_at(), None);
        assert_eq!(schedule.disabled_reason(), Some("Failed 3 times in a row"));
    }

    #[test]
    fn success_resets_and_awaiting_approval_keeps_the_count() {
        let schedule = schedule().recorded(&failed(now())).recorded(&failed(now()));
        let waiting = schedule
            .clone()
            .recorded(&RunRecord::new(RunOutcome::AwaitingApproval, now()));
        assert_eq!(waiting.consecutive_failures(), 2);
        assert_eq!(waiting.last_status(), Some(RunStatus::AwaitingApproval));
        assert_eq!(waiting.last_error(), None);
        let ok = schedule.recorded(&RunRecord::new(RunOutcome::Ok, now() + Duration::hours(1)));
        assert_eq!(ok.consecutive_failures(), 0);
        assert_eq!(ok.last_status(), Some(RunStatus::Ok));
        assert_eq!(ok.last_run_at(), Some(now() + Duration::hours(1)));
    }

    #[test]
    fn a_run_without_its_model_pauses_the_schedule_at_once() {
        let paused = || RunOutcome::Paused {
            reason: MODEL_REMOVED.to_owned(),
            message: "This schedule's model was removed.".to_owned(),
        };
        let schedule = schedule().recorded(&RunRecord::new(paused(), now()));
        assert!(!schedule.enabled());
        assert_eq!(schedule.next_run_at(), None);
        assert_eq!(schedule.disabled_reason(), Some(MODEL_REMOVED));
        assert_eq!(schedule.last_status(), Some(RunStatus::Error));
        assert_eq!(
            schedule.last_error(),
            Some("This schedule's model was removed.")
        );
        let off = schedule
            .disabled(None)
            .recorded(&RunRecord::new(paused(), now()));
        assert_eq!(
            off.disabled_reason(),
            None,
            "a schedule already off stays as it was"
        );
    }

    #[test]
    fn a_run_that_could_not_start_fails_with_a_readable_reason() {
        let error = DomainError::validation("Too many files.");
        assert_eq!(
            RunOutcome::not_started(&error),
            RunOutcome::Failed("Too many files.".to_owned())
        );
        let error = DomainError::internal("database.chats.insert.failed", "boom");
        assert_eq!(
            RunOutcome::not_started(&error),
            RunOutcome::Failed(START_FAILED.to_owned()),
            "never a dotted key"
        );
    }

    #[test]
    fn a_failure_on_a_paused_schedule_keeps_its_reason() {
        let schedule = schedule()
            .disabled(None)
            .recorded(&failed(now()))
            .recorded(&failed(now()))
            .recorded(&failed(now()));
        assert_eq!(schedule.disabled_reason(), None, "the user turned it off");
    }

    #[test]
    fn parses_text_values() {
        for (text, value) in [("deny", OnAsk::Deny), ("pause", OnAsk::Pause)] {
            assert_eq!(text.parse::<OnAsk>(), Ok(value));
            assert_eq!(value.as_str(), text);
        }
        assert_eq!("assistant".parse::<CreatedBy>(), Ok(CreatedBy::Assistant));
        assert_eq!(
            "awaiting_approval".parse::<RunStatus>(),
            Ok(RunStatus::AwaitingApproval)
        );
        assert!("later".parse::<OnAsk>().is_err());
    }
}
