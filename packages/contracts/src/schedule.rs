//! `/schedules`: prompts that run on a cron rule. Each firing creates a new chat; a schedule's
//! runs are `GET /chats?scheduleId=`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::shared::ReasoningLevel;

/// What an unattended run does when a tool asks for approval.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum OnAsk {
    /// "Deny and continue": the call is denied and the run goes on.
    #[default]
    Deny,
    /// "Pause until I approve": the run stops awaiting approval.
    Pause,
}

/// Who made the schedule. The assistant creates schedules through the built-in schedule tools.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum CreatedBy {
    User,
    Assistant,
}

/// How the last run ended.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Ok,
    Error,
    AwaitingApproval,
}

/// A schedule.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ScheduleResponse {
    pub id: Uuid,
    /// `null` for a standalone schedule.
    pub project_id: Option<Uuid>,
    /// `null` after the model was deleted (the schedule is then turned off).
    pub model_id: Option<Uuid>,
    /// The chat it was scheduled from, if any.
    pub source_chat_id: Option<Uuid>,
    pub name: String,
    /// Five fields: minute, hour, day of month, month, day of week.
    pub cron: String,
    /// IANA time zone the rule runs in.
    pub timezone: String,
    /// The rule in plain words.
    pub description: String,
    pub prompt: String,
    /// Used only without a project.
    pub instructions: String,
    /// `null`: the project's level, then the model's default.
    pub reasoning: Option<ReasoningLevel>,
    pub on_ask: OnAsk,
    pub created_by: CreatedBy,
    pub enabled: bool,
    /// `null` while turned off.
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_status: Option<RunStatus>,
    /// Human-readable.
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    /// Why the schedule turned itself off, for example "Model removed" or "Failed 3 times in a
    /// row".
    pub disabled_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Query of `GET /schedules`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, IntoParams, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct ScheduleListQuery {
    /// Only this project's schedules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Uuid>,
}

/// `POST /schedules`. The shortest gap between fires is 15 minutes, and a user has at most 20
/// enabled schedules.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct CreateScheduleRequest {
    pub name: String,
    pub cron: String,
    pub timezone: String,
    pub prompt: String,
    /// Absent for a standalone schedule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<Uuid>,
    /// Used only without a project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningLevel>,
    /// Default `deny`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_ask: Option<OnAsk>,
}

/// `PATCH /schedules/{id}`: only the given fields change. A new rule moves the next run;
/// turning a schedule on clears its failure count and pause reason.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct UpdateScheduleRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<Uuid>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::patch::nullable"
    )]
    pub reasoning: Option<Option<ReasoningLevel>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_ask: Option<OnAsk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// `POST /schedules/preview`: what a rule means before saving it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SchedulePreviewRequest {
    pub cron: String,
    pub timezone: String,
}

/// Answer of `POST /schedules/preview`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SchedulePreviewResponse {
    /// The rule in plain words.
    pub description: String,
    pub timezone: String,
    /// The next 3 fires.
    pub next_runs: Vec<DateTime<Utc>>,
}

/// `POST /schedules/{id}/run`: the run started, unattended, in a new chat. Attach to it with
/// `GET /chats/{chatId}/stream`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRunResponse {
    pub chat_id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_are_snake_case() {
        assert_eq!(
            serde_json::to_value([RunStatus::Ok, RunStatus::AwaitingApproval]).expect("serializes"),
            serde_json::json!(["ok", "awaiting_approval"])
        );
        assert_eq!(
            serde_json::to_value((OnAsk::Pause, CreatedBy::Assistant)).expect("serializes"),
            serde_json::json!(["pause", "assistant"])
        );
    }

    #[test]
    fn create_reads_a_standalone_schedule() {
        let request: CreateScheduleRequest = serde_json::from_value(serde_json::json!({
            "name": "Morning news",
            "cron": "0 7 * * 1-5",
            "timezone": "Europe/Berlin",
            "prompt": "Summarize the news.",
            "instructions": "Be brief.",
        }))
        .expect("valid body");
        assert_eq!(request.project_id, None);
        assert_eq!(request.on_ask, None);
        assert_eq!(request.instructions.as_deref(), Some("Be brief."));
    }

    #[test]
    fn update_clears_reasoning_with_null() {
        let request: UpdateScheduleRequest =
            serde_json::from_value(serde_json::json!({ "reasoning": null, "enabled": false }))
                .expect("valid body");
        assert_eq!(request.reasoning, Some(None));
        assert_eq!(request.enabled, Some(false));
        assert_eq!(request.cron, None);
    }
}
