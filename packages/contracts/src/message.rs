//! Messages of a chat (`GET /chats/{id}/messages`) and the values the stream events share with
//! them. Provider-opaque data (replay state, provider call ids, tool aliases) never leaves the
//! server.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

/// Who wrote a message. A `tool` message holds the results of one round of tool calls, in call
/// order.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    /// Still being written by a live run.
    Streaming,
    Complete,
    /// Tool calls wait for the user's decision.
    AwaitingApproval,
    /// Stopped by the user or a shutdown; the partial answer is kept.
    Cancelled,
    /// The run failed; `error` says why.
    Error,
    /// The model declined; `error` holds its explanation.
    Refused,
}

/// Why an answer ended.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    ToolCalls,
    /// The output limit: the UI offers Continue.
    Length,
    Refusal,
    Error,
    Other,
    /// The model asked for tools in the last allowed round: the UI offers Continue.
    ToolLimit,
}

/// The outcome of one tool call.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Ok,
    Error,
    Denied,
    Cancelled,
    Timeout,
}

/// Where a tool call stands with the user.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum Approval {
    /// Ran without asking.
    Auto,
    /// Waits for a decision.
    Pending,
    Approved,
    Denied,
}

/// Token counts the provider reported for one answer; any may be missing.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub reasoning_tokens: Option<u32>,
    pub cache_read_tokens: Option<u32>,
    pub cache_write_tokens: Option<u32>,
}

/// One piece of a tool result. Tool output renders as preformatted text, never as Markdown or
/// HTML.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ResultContent {
    Text {
        text: String,
    },
    /// An image the tool returned, saved as a chat file (`GET /files/{fileId}/content`).
    #[serde(rename_all = "camelCase")]
    Image {
        file_id: Uuid,
    },
}

/// One part of a message, tagged by `type`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MessagePartDto {
    Text {
        text: String,
    },
    /// An attachment (a chat file).
    #[serde(rename_all = "camelCase")]
    File {
        file_id: Uuid,
        name: String,
        media_type: String,
    },
    /// Recorded or spoken audio, stored as a chat file.
    #[serde(rename_all = "camelCase")]
    Audio {
        file_id: Uuid,
        format: String,
        /// What the provider transcribed or spoke, when it said.
        transcript: Option<String>,
        duration_ms: Option<u32>,
    },
    /// A reasoning summary; display only.
    Reasoning {
        text: String,
    },
    /// A tool call of an assistant message.
    #[serde(rename_all = "camelCase")]
    ToolCall {
        call_id: Uuid,
        /// `null` for a built-in tool.
        server_id: Option<Uuid>,
        /// The original name, never the alias sent to the model.
        tool_name: String,
        /// The parsed arguments, or the raw text when they are not a JSON object.
        arguments: Value,
        /// The arguments finished streaming.
        complete: bool,
        approval: Approval,
        /// The reason a denial passed to the model.
        deny_reason: Option<String>,
    },
    /// A result in a `tool` message.
    #[serde(rename_all = "camelCase")]
    ToolResult {
        call_id: Uuid,
        status: ToolStatus,
        is_error: bool,
        content: Vec<ResultContent>,
        /// Cut to the result size limit, with a visible note.
        truncated: bool,
    },
}

/// A stored message.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct MessageResponse {
    pub id: Uuid,
    pub chat_id: Uuid,
    pub role: Role,
    pub status: MessageStatus,
    /// Assistant messages only.
    pub finish_reason: Option<FinishReason>,
    /// The model that wrote an assistant message.
    pub model_id: Option<Uuid>,
    pub parts: Vec<MessagePartDto>,
    pub usage: Option<Usage>,
    /// Human-readable: why the run failed, or the model's refusal.
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_are_tagged_by_type_with_camel_case_fields() {
        let call = MessagePartDto::ToolCall {
            call_id: Uuid::nil(),
            server_id: None,
            tool_name: "list_schedules".to_owned(),
            arguments: serde_json::json!({}),
            complete: true,
            approval: Approval::Auto,
            deny_reason: None,
        };
        assert_eq!(
            serde_json::to_value(&call).expect("serializes"),
            serde_json::json!({
                "type": "toolCall",
                "callId": "00000000-0000-0000-0000-000000000000",
                "serverId": null,
                "toolName": "list_schedules",
                "arguments": {},
                "complete": true,
                "approval": "auto",
                "denyReason": null,
            })
        );
        let result = MessagePartDto::ToolResult {
            call_id: Uuid::nil(),
            status: ToolStatus::Timeout,
            is_error: true,
            content: vec![
                ResultContent::Text {
                    text: "The call timed out.".to_owned(),
                },
                ResultContent::Image {
                    file_id: Uuid::nil(),
                },
            ],
            truncated: false,
        };
        assert_eq!(
            serde_json::to_value(&result).expect("serializes"),
            serde_json::json!({
                "type": "toolResult",
                "callId": "00000000-0000-0000-0000-000000000000",
                "status": "timeout",
                "isError": true,
                "content": [
                    { "type": "text", "text": "The call timed out." },
                    { "type": "image", "fileId": "00000000-0000-0000-0000-000000000000" },
                ],
                "truncated": false,
            })
        );
    }

    #[test]
    fn file_and_audio_parts_use_camel_case() {
        let audio = MessagePartDto::Audio {
            file_id: Uuid::nil(),
            format: "wav".to_owned(),
            transcript: Some("Hello".to_owned()),
            duration_ms: Some(1500),
        };
        let json = serde_json::to_value(&audio).expect("serializes");
        assert_eq!(json["type"], "audio");
        assert_eq!(json["durationMs"], 1500);
        assert_eq!(json["fileId"], "00000000-0000-0000-0000-000000000000");
        let file: MessagePartDto = serde_json::from_value(serde_json::json!({
            "type": "file",
            "fileId": "00000000-0000-0000-0000-000000000000",
            "name": "report.pdf",
            "mediaType": "application/pdf",
        }))
        .expect("valid part");
        assert!(matches!(file, MessagePartDto::File { .. }));
    }

    #[test]
    fn statuses_and_reasons_are_snake_case() {
        assert_eq!(
            serde_json::to_value((MessageStatus::AwaitingApproval, FinishReason::ToolLimit))
                .expect("serializes"),
            serde_json::json!(["awaiting_approval", "tool_limit"])
        );
        assert_eq!(
            serde_json::to_value((FinishReason::ToolCalls, Role::Tool, Approval::Pending))
                .expect("serializes"),
            serde_json::json!(["tool_calls", "tool", "pending"])
        );
    }
}
