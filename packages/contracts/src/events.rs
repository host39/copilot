//! Server-sent events. Each event's data is one JSON object tagged by `type`.
//!
//! - [`ChatStreamEvent`]: the streaming chat routes and `GET /chats/{id}/stream`. Every event
//!   carries the run's `seq`, so a client that reattaches (the replay, then live events) drops
//!   the events it already has. A run always ends with `done`.
//! - [`UserEvent`]: `GET /events`, one stream per user, so open clients refresh the sidebar
//!   without polling.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::message::{FinishReason, MessageStatus, ResultContent, Role, ToolStatus, Usage};

/// The kind of a message part that starts streaming.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub enum PartKind {
    Text,
    Reasoning,
    ToolCall,
    Audio,
}

/// Why the model declined to answer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct Refusal {
    pub category: Option<String>,
    /// Shown instead of the partial answer.
    pub explanation: String,
}

/// One event of a chat run.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(tag = "type")]
pub enum ChatStreamEvent {
    /// A message starts (the assistant's answer, or the `tool` message of a round).
    #[serde(rename = "message.started", rename_all = "camelCase")]
    MessageStarted {
        seq: u64,
        message_id: Uuid,
        role: Role,
        /// The first message the model sees; `null` when the window starts at the first message
        /// of the chat. Earlier messages get the "Earlier messages not sent" divider.
        window_start_id: Option<Uuid>,
    },
    /// `index` is the part's position in the message.
    #[serde(rename = "part.started", rename_all = "camelCase")]
    PartStarted {
        seq: u64,
        message_id: Uuid,
        index: u32,
        kind: PartKind,
    },
    #[serde(rename = "text.delta", rename_all = "camelCase")]
    TextDelta {
        seq: u64,
        message_id: Uuid,
        index: u32,
        text: String,
    },
    #[serde(rename = "reasoning.delta", rename_all = "camelCase")]
    ReasoningDelta {
        seq: u64,
        message_id: Uuid,
        index: u32,
        text: String,
    },
    /// Spoken reply audio, played as it arrives.
    #[serde(rename = "audio.delta", rename_all = "camelCase")]
    AudioDelta {
        seq: u64,
        message_id: Uuid,
        /// Base64 audio in `format` (`pcm16`).
        data: String,
        format: String,
    },
    /// A tool call whose arguments are complete.
    #[serde(rename = "tool.requested", rename_all = "camelCase")]
    ToolRequested {
        seq: u64,
        message_id: Uuid,
        /// Copilot's id for the call; decisions name it.
        call_id: Uuid,
        /// `null` for a built-in tool.
        server_id: Option<Uuid>,
        /// The original name, never the alias.
        tool_name: String,
        /// The tool's display title, else its name.
        title: String,
        /// The parsed arguments, or the raw text when they are not a JSON object.
        arguments: Value,
        /// The run pauses for a decision (`message.finished` follows with `awaiting_approval`).
        needs_approval: bool,
    },
    /// A result, written to the `tool` message `messageId`.
    #[serde(rename = "tool.finished", rename_all = "camelCase")]
    ToolFinished {
        seq: u64,
        message_id: Uuid,
        call_id: Uuid,
        status: ToolStatus,
        /// Cut for display; the stored result may be longer.
        output: Vec<ResultContent>,
        truncated: bool,
    },
    #[serde(rename = "message.finished", rename_all = "camelCase")]
    MessageFinished {
        seq: u64,
        message_id: Uuid,
        status: MessageStatus,
        finish_reason: Option<FinishReason>,
        /// Replaces the streamed text when the model declined.
        refusal: Option<Refusal>,
        usage: Option<Usage>,
    },
    /// The run failed. `message` is human-readable; `code` is the machine code (for example
    /// `PROVIDER_AUTH`).
    #[serde(rename = "error", rename_all = "camelCase")]
    Error {
        seq: u64,
        message: String,
        code: String,
    },
    /// The last event of every run.
    #[serde(rename = "done", rename_all = "camelCase")]
    Done { seq: u64 },
}

impl ChatStreamEvent {
    /// The event's position in its run.
    pub fn seq(&self) -> u64 {
        match self {
            Self::MessageStarted { seq, .. }
            | Self::PartStarted { seq, .. }
            | Self::TextDelta { seq, .. }
            | Self::ReasoningDelta { seq, .. }
            | Self::AudioDelta { seq, .. }
            | Self::ToolRequested { seq, .. }
            | Self::ToolFinished { seq, .. }
            | Self::MessageFinished { seq, .. }
            | Self::Error { seq, .. }
            | Self::Done { seq } => *seq,
        }
    }
}

/// One event of the caller's `GET /events` stream.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(tag = "type")]
pub enum UserEvent {
    /// A chat changed (title, settings, or new messages).
    #[serde(rename = "chat.updated", rename_all = "camelCase")]
    ChatUpdated { chat_id: Uuid },
    #[serde(rename = "run.started", rename_all = "camelCase")]
    RunStarted { chat_id: Uuid },
    /// `status` is the run's last answer.
    #[serde(rename = "run.finished", rename_all = "camelCase")]
    RunFinished {
        chat_id: Uuid,
        status: MessageStatus,
    },
    /// A tool call in the chat waits for a decision.
    #[serde(rename = "approval.pending", rename_all = "camelCase")]
    ApprovalPending { chat_id: Uuid, message_id: Uuid },
    /// A schedule changed (a run, a failure, or a pause).
    #[serde(rename = "schedule.updated", rename_all = "camelCase")]
    ScheduleUpdated { schedule_id: Uuid },
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "00000000-0000-0000-0000-000000000000";

    /// One event of every type, each with its `seq`.
    fn events() -> Vec<ChatStreamEvent> {
        let id = Uuid::nil();
        vec![
            ChatStreamEvent::MessageStarted {
                seq: 1,
                message_id: id,
                role: Role::Assistant,
                window_start_id: None,
            },
            ChatStreamEvent::PartStarted {
                seq: 2,
                message_id: id,
                index: 0,
                kind: PartKind::ToolCall,
            },
            ChatStreamEvent::TextDelta {
                seq: 3,
                message_id: id,
                index: 0,
                text: "Hi".to_owned(),
            },
            ChatStreamEvent::ReasoningDelta {
                seq: 4,
                message_id: id,
                index: 1,
                text: "Thinking".to_owned(),
            },
            ChatStreamEvent::AudioDelta {
                seq: 5,
                message_id: id,
                data: "AAA=".to_owned(),
                format: "pcm16".to_owned(),
            },
            ChatStreamEvent::ToolRequested {
                seq: 6,
                message_id: id,
                call_id: id,
                server_id: None,
                tool_name: "schedule_prompt".to_owned(),
                title: "Schedule a prompt".to_owned(),
                arguments: serde_json::json!({ "name": "News" }),
                needs_approval: true,
            },
            ChatStreamEvent::ToolFinished {
                seq: 7,
                message_id: id,
                call_id: id,
                status: ToolStatus::Ok,
                output: vec![ResultContent::Text {
                    text: "done".to_owned(),
                }],
                truncated: false,
            },
            ChatStreamEvent::MessageFinished {
                seq: 8,
                message_id: id,
                status: MessageStatus::Refused,
                finish_reason: Some(FinishReason::Refusal),
                refusal: Some(Refusal {
                    category: None,
                    explanation: "The model declined to answer.".to_owned(),
                }),
                usage: Some(Usage {
                    input_tokens: Some(10),
                    ..Usage::default()
                }),
            },
            ChatStreamEvent::Error {
                seq: 9,
                message: "Key rejected: open provider settings.".to_owned(),
                code: "PROVIDER_AUTH".to_owned(),
            },
            ChatStreamEvent::Done { seq: 10 },
        ]
    }

    #[test]
    fn every_event_has_its_dotted_type_and_seq() {
        let types: Vec<(String, u64)> = events()
            .iter()
            .map(|event| {
                let json = serde_json::to_value(event).expect("serializes");
                assert_eq!(json["seq"], event.seq(), "{json}");
                (
                    json["type"].as_str().expect("a type").to_owned(),
                    event.seq(),
                )
            })
            .collect();
        let expected = [
            "message.started",
            "part.started",
            "text.delta",
            "reasoning.delta",
            "audio.delta",
            "tool.requested",
            "tool.finished",
            "message.finished",
            "error",
            "done",
        ];
        assert_eq!(
            types
                .iter()
                .map(|(kind, _)| kind.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            types.iter().map(|(_, seq)| *seq).collect::<Vec<_>>(),
            (1..=10).collect::<Vec<u64>>()
        );
    }

    #[test]
    fn events_round_trip() {
        for event in events() {
            let json = serde_json::to_value(&event).expect("serializes");
            let read: ChatStreamEvent = serde_json::from_value(json).expect("deserializes");
            assert_eq!(read, event);
        }
    }

    #[test]
    fn payloads_use_camel_case() {
        let events = events();
        assert_eq!(
            serde_json::to_value(&events[0]).expect("serializes"),
            serde_json::json!({
                "type": "message.started",
                "seq": 1,
                "messageId": ID,
                "role": "assistant",
                "windowStartId": null,
            })
        );
        assert_eq!(
            serde_json::to_value(&events[5]).expect("serializes"),
            serde_json::json!({
                "type": "tool.requested",
                "seq": 6,
                "messageId": ID,
                "callId": ID,
                "serverId": null,
                "toolName": "schedule_prompt",
                "title": "Schedule a prompt",
                "arguments": { "name": "News" },
                "needsApproval": true,
            })
        );
        let finished = serde_json::to_value(&events[7]).expect("serializes");
        assert_eq!(finished["finishReason"], "refusal");
        assert_eq!(finished["status"], "refused");
        assert_eq!(finished["usage"]["inputTokens"], 10);
        assert_eq!(finished["refusal"]["category"], serde_json::Value::Null);
        assert_eq!(
            serde_json::to_value(&events[1]).expect("serializes")["kind"],
            "toolCall"
        );
        assert_eq!(
            serde_json::to_value(&events[9]).expect("serializes"),
            serde_json::json!({ "type": "done", "seq": 10 })
        );
    }

    #[test]
    fn user_events_have_dotted_types() {
        let id = Uuid::nil();
        let events = [
            UserEvent::ChatUpdated { chat_id: id },
            UserEvent::RunStarted { chat_id: id },
            UserEvent::RunFinished {
                chat_id: id,
                status: MessageStatus::AwaitingApproval,
            },
            UserEvent::ApprovalPending {
                chat_id: id,
                message_id: id,
            },
            UserEvent::ScheduleUpdated { schedule_id: id },
        ];
        let json: Vec<serde_json::Value> = events
            .iter()
            .map(|event| serde_json::to_value(event).expect("serializes"))
            .collect();
        assert_eq!(
            json,
            [
                serde_json::json!({ "type": "chat.updated", "chatId": ID }),
                serde_json::json!({ "type": "run.started", "chatId": ID }),
                serde_json::json!({
                    "type": "run.finished",
                    "chatId": ID,
                    "status": "awaiting_approval",
                }),
                serde_json::json!({ "type": "approval.pending", "chatId": ID, "messageId": ID }),
                serde_json::json!({ "type": "schedule.updated", "scheduleId": ID }),
            ]
        );
    }
}
