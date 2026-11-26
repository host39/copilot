//! `/chats` and `/search`. The streaming routes (`POST /chats/{id}/messages`, `/regenerate`,
//! `/retry`, `/messages/{messageId}/decisions`, and `GET /chats/{id}/stream`) answer with
//! server-sent events whose data is a [`crate::ChatStreamEvent`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::shared::{BuiltinTool, ReasoningLevel};

/// A tool server added to (`enabled: true`) or removed from (`false`) the project's servers in
/// one chat.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ToolOverride {
    pub server_id: Uuid,
    pub enabled: bool,
}

/// A server's tool approved for the whole chat ("Approve for this chat"), by original name.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ApprovedTool {
    pub server_id: Uuid,
    pub name: String,
}

/// A chat with its settings (details and writes).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ChatResponse {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    /// `null` shows the model picker ("Choose a model to continue").
    pub model_id: Option<Uuid>,
    /// The schedule whose run created the chat.
    pub schedule_id: Option<Uuid>,
    pub title: String,
    /// Appended after the project's instructions; the system prompt of a standalone chat.
    pub instructions: String,
    /// `null`: the project's level, then the model's default.
    pub reasoning: Option<ReasoningLevel>,
    /// `null` inherits the project's (none for a standalone chat).
    pub builtin_tools: Option<Vec<BuiltinTool>>,
    pub tool_overrides: Vec<ToolOverride>,
    pub approved_tools: Vec<ApprovedTool>,
    pub pinned: bool,
    /// "Speak replies", for models that return audio.
    pub voice_replies: bool,
    pub last_read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// Moves with every message, never with settings changes.
    pub updated_at: DateTime<Utc>,
}

/// A row of the sidebar or the Scheduled list.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ChatSummaryResponse {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub model_id: Option<Uuid>,
    pub schedule_id: Option<Uuid>,
    pub title: String,
    pub pinned: bool,
    /// Changed since the user last opened it.
    pub unread: bool,
    /// Its latest answer waits for a tool decision.
    pub awaiting_approval: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Query of `GET /chats`: most recently updated first, one [`crate::PageResponse`] at a time.
#[derive(Clone, Debug, Default, Deserialize, Eq, IntoParams, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct ChatListQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Uuid>,
    /// The runs of one schedule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
    /// `true`: only schedule runs (the Scheduled list); `false`: only the others (Recent chats).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled: Option<bool>,
    /// The `next` of the previous page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Page size; the server caps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// `POST /chats`: the new chat screen's draft, sent with the first message so empty chats never
/// exist. The title comes from the first message.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct CreateChatRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningLevel>,
    /// Absent inherits the project's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin_tools: Option<Vec<BuiltinTool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_overrides: Option<Vec<ToolOverride>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_replies: Option<bool>,
}

/// `PATCH /chats/{id}`: only the given fields change; `null` clears the nullable ones. Settings
/// apply from the next user message.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct UpdateChatRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Moves the chat into a project, or out of one with `null`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::patch::nullable"
    )]
    pub project_id: Option<Option<Uuid>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::patch::nullable"
    )]
    pub model_id: Option<Option<Uuid>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::patch::nullable"
    )]
    pub reasoning: Option<Option<ReasoningLevel>>,
    /// `null` inherits the project's again.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::patch::nullable"
    )]
    pub builtin_tools: Option<Option<Vec<BuiltinTool>>>,
    /// Replaces every override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_overrides: Option<Vec<ToolOverride>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_replies: Option<bool>,
}

/// `POST /chats/{id}/messages`: a new user message, or an edit. Pending tool calls of the chat
/// are denied first ("Skipped by user.").
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct SendMessageRequest {
    pub text: String,
    /// Chat files uploaded for this message (`POST /chats/{id}/files`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_ids: Vec<Uuid>,
    /// Edit and resend: this user message and every later one are replaced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_from: Option<Uuid>,
}

/// The user's answer to one pending tool call.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Approve,
    Deny,
}

/// How far an approval reaches.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum DecisionScope {
    /// This call only.
    #[default]
    Once,
    /// This call, and later calls of the same server tool in this chat.
    Chat,
}

/// A decision on one pending call, applied to its stored arguments exactly once.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct CallDecision {
    pub call_id: Uuid,
    pub decision: Decision,
    /// Default `once`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<DecisionScope>,
    /// Passed to the model with a denial.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `POST /chats/{id}/messages/{messageId}/decisions`: decisions on exactly the pending calls of
/// the chat's latest answer while it awaits approval. The answer streams the continuation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct DecisionsRequest {
    pub decisions: Vec<CallDecision>,
}

/// Query of `GET /search`: chat titles and message text.
#[derive(Clone, Debug, Default, Deserialize, Eq, IntoParams, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct SearchQuery {
    pub q: String,
    /// The server caps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// A search match.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SearchHitResponse {
    pub chat_id: Uuid,
    pub chat_title: String,
    /// `null` when the title matched.
    pub message_id: Option<Uuid>,
    /// Plain text around the match.
    pub snippet: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_moves_a_chat_out_of_its_project_with_null() {
        let request: UpdateChatRequest = serde_json::from_value(serde_json::json!({
            "projectId": null,
            "builtinTools": null,
            "pinned": true,
        }))
        .expect("valid body");
        assert_eq!(request.project_id, Some(None));
        assert_eq!(request.builtin_tools, Some(None));
        assert_eq!(request.model_id, None);
        assert_eq!(request.pinned, Some(true));
        let tools: UpdateChatRequest =
            serde_json::from_value(serde_json::json!({ "builtinTools": ["schedules"] }))
                .expect("valid body");
        assert_eq!(
            tools.builtin_tools,
            Some(Some(vec![BuiltinTool::Schedules]))
        );
    }

    #[test]
    fn send_defaults_to_no_files_and_no_edit() {
        let request: SendMessageRequest =
            serde_json::from_value(serde_json::json!({ "text": "Hello" })).expect("valid body");
        assert!(request.file_ids.is_empty());
        assert_eq!(request.edit_from, None);
        assert_eq!(
            serde_json::to_value(&request).expect("serializes"),
            serde_json::json!({ "text": "Hello" })
        );
    }

    #[test]
    fn decisions_read_snake_case_values() {
        let request: DecisionsRequest = serde_json::from_value(serde_json::json!({
            "decisions": [
                {
                    "callId": "00000000-0000-0000-0000-000000000000",
                    "decision": "approve",
                    "scope": "chat",
                },
                {
                    "callId": "00000000-0000-0000-0000-000000000000",
                    "decision": "deny",
                    "reason": "Not now.",
                },
            ],
        }))
        .expect("valid body");
        assert_eq!(request.decisions[0].decision, Decision::Approve);
        assert_eq!(request.decisions[0].scope, Some(DecisionScope::Chat));
        assert_eq!(request.decisions[1].scope, None);
        assert_eq!(request.decisions[1].reason.as_deref(), Some("Not now."));
        assert_eq!(DecisionScope::default(), DecisionScope::Once);
    }

    #[test]
    fn list_query_reads_the_cursor() {
        let query: ChatListQuery = serde_json::from_value(serde_json::json!({
            "scheduled": false,
            "cursor": "opaque",
        }))
        .expect("valid query");
        assert_eq!(query.scheduled, Some(false));
        assert_eq!(query.cursor.as_deref(), Some("opaque"));
    }
}
