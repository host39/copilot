//! Chats, messages, and run events as the API sends them. Provider-opaque data (replay state,
//! provider call ids, tool aliases) never leaves here, and tool output is cut for display.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use copilot_contracts::{
    ApprovedTool, ChatResponse, ChatStreamEvent, ChatSummaryResponse, MessagePartDto,
    MessageResponse, ResultContent, ToolOverride,
};
use copilot_core::{
    CallId, Chat, ChatSummary, Message, MessageId, Part, ResultContent as CoreContent,
    ToolServerId, TurnEvent,
};
use serde_json::Value;

use crate::routes::convert::ToDto;

/// Longest tool result text a stream event shows, in bytes; the stored result may be longer
/// (up to `MCP_RESULT_MAX_BYTES`), and the messages route returns it whole.
pub const DISPLAY_MAX_BYTES: usize = 8_192;

pub fn chat(chat: &Chat) -> ChatResponse {
    ChatResponse {
        id: chat.id().uuid(),
        project_id: chat.project_id().map(|id| id.uuid()),
        model_id: chat.model_id().map(|id| id.uuid()),
        schedule_id: chat.schedule_id().map(|id| id.uuid()),
        title: chat.title().to_owned(),
        instructions: chat.instructions().to_owned(),
        reasoning: chat.reasoning().map(ToDto::to_dto),
        builtin_tools: chat
            .builtin_tools()
            .map(|tools| tools.iter().map(|tool| tool.to_dto()).collect()),
        tool_overrides: chat
            .tool_overrides()
            .iter()
            .map(|row| ToolOverride {
                server_id: row.server_id.uuid(),
                enabled: row.enabled,
            })
            .collect(),
        approved_tools: chat
            .approved_tools()
            .iter()
            .map(|tool| ApprovedTool {
                server_id: tool.server_id.uuid(),
                name: tool.name.clone(),
            })
            .collect(),
        pinned: chat.pinned(),
        voice_replies: chat.voice_replies(),
        last_read_at: chat.last_read_at(),
        created_at: chat.created_at(),
        updated_at: chat.updated_at(),
    }
}

/// A row of the sidebar or the Scheduled list.
pub fn summary(row: &ChatSummary) -> ChatSummaryResponse {
    let chat = &row.chat;
    ChatSummaryResponse {
        id: chat.id().uuid(),
        project_id: chat.project_id().map(|id| id.uuid()),
        model_id: chat.model_id().map(|id| id.uuid()),
        schedule_id: chat.schedule_id().map(|id| id.uuid()),
        title: chat.title().to_owned(),
        pinned: chat.pinned(),
        unread: row.unread,
        awaiting_approval: row.awaiting_approval,
        created_at: chat.created_at(),
        updated_at: chat.updated_at(),
    }
}

pub fn message(message: &Message) -> MessageResponse {
    MessageResponse {
        id: message.id().uuid(),
        chat_id: message.chat_id().uuid(),
        role: message.role().to_dto(),
        status: message.status().to_dto(),
        finish_reason: message.finish_reason().map(ToDto::to_dto),
        model_id: message.model_id().map(|id| id.uuid()),
        parts: message.parts().iter().map(part).collect(),
        usage: message.usage().copied().map(ToDto::to_dto),
        error: message.error().map(ToOwned::to_owned),
        created_at: message.created_at(),
        updated_at: message.updated_at(),
    }
}

fn part(part: &Part) -> MessagePartDto {
    match part {
        Part::Text { text } => MessagePartDto::Text { text: text.clone() },
        Part::File {
            file_id,
            name,
            media_type,
        } => MessagePartDto::File {
            file_id: file_id.uuid(),
            name: name.clone(),
            media_type: media_type.clone(),
        },
        Part::Audio {
            file_id,
            format,
            transcript,
            duration_ms,
        } => MessagePartDto::Audio {
            file_id: file_id.uuid(),
            format: format.clone(),
            transcript: transcript.clone(),
            duration_ms: *duration_ms,
        },
        Part::Reasoning { text } => MessagePartDto::Reasoning { text: text.clone() },
        Part::ToolCall(call) => MessagePartDto::ToolCall {
            call_id: call.id.uuid(),
            server_id: call.server_id.map(ToolServerId::uuid),
            tool_name: call.tool_name.clone(),
            arguments: call
                .arguments
                .clone()
                .unwrap_or_else(|| Value::String(call.arguments_raw.clone())),
            complete: call.complete,
            approval: call.approval.to_dto(),
            deny_reason: call.deny_reason.clone(),
        },
        Part::ToolResult(result) => MessagePartDto::ToolResult {
            call_id: result.call_id.uuid(),
            status: result.status.to_dto(),
            is_error: result.is_error,
            content: result.content.iter().map(ToDto::to_dto).collect(),
            truncated: result.truncated,
        },
    }
}

/// A run event with its `seq`.
pub fn event(event: TurnEvent, seq: u64) -> ChatStreamEvent {
    let id = MessageId::uuid;
    match event {
        TurnEvent::MessageStarted {
            message_id,
            role,
            window_start_id,
        } => ChatStreamEvent::MessageStarted {
            seq,
            message_id: id(message_id),
            role: role.to_dto(),
            window_start_id: window_start_id.map(id),
        },
        TurnEvent::PartStarted {
            message_id,
            index,
            kind,
        } => ChatStreamEvent::PartStarted {
            seq,
            message_id: id(message_id),
            index,
            kind: kind.to_dto(),
        },
        TurnEvent::TextDelta {
            message_id,
            index,
            text,
        } => ChatStreamEvent::TextDelta {
            seq,
            message_id: id(message_id),
            index,
            text,
        },
        TurnEvent::ReasoningDelta {
            message_id,
            index,
            text,
        } => ChatStreamEvent::ReasoningDelta {
            seq,
            message_id: id(message_id),
            index,
            text,
        },
        TurnEvent::AudioDelta {
            message_id,
            data,
            format,
        } => ChatStreamEvent::AudioDelta {
            seq,
            message_id: id(message_id),
            data: STANDARD.encode(&data),
            format,
        },
        TurnEvent::ToolRequested {
            message_id,
            call_id,
            server_id,
            tool_name,
            title,
            arguments,
            needs_approval,
        } => ChatStreamEvent::ToolRequested {
            seq,
            message_id: id(message_id),
            call_id: CallId::uuid(call_id),
            server_id: server_id.map(ToolServerId::uuid),
            tool_name,
            title,
            arguments,
            needs_approval,
        },
        TurnEvent::ToolFinished {
            message_id,
            call_id,
            status,
            output,
            truncated,
        } => {
            let (output, cut) = display(&output);
            ChatStreamEvent::ToolFinished {
                seq,
                message_id: id(message_id),
                call_id: CallId::uuid(call_id),
                status: status.to_dto(),
                output,
                truncated: truncated || cut,
            }
        }
        TurnEvent::MessageFinished {
            message_id,
            status,
            finish_reason,
            refusal,
            usage,
        } => ChatStreamEvent::MessageFinished {
            seq,
            message_id: id(message_id),
            status: status.to_dto(),
            finish_reason: finish_reason.map(ToDto::to_dto),
            refusal: refusal.map(ToDto::to_dto),
            usage: usage.map(ToDto::to_dto),
        },
        TurnEvent::Error { message, code } => ChatStreamEvent::Error { seq, message, code },
        TurnEvent::Done => ChatStreamEvent::Done { seq },
    }
}

/// Tool output cut to [`DISPLAY_MAX_BYTES`] of text in total (on a character boundary), and
/// whether anything was cut.
fn display(output: &[CoreContent]) -> (Vec<ResultContent>, bool) {
    let mut left = DISPLAY_MAX_BYTES;
    let mut cut = false;
    let mut shown = Vec::with_capacity(output.len());
    for content in output {
        match content {
            CoreContent::Text(text) if text.len() > left => {
                let mut end = left;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                left = 0;
                cut = true;
                if end > 0 {
                    shown.push(ResultContent::Text {
                        text: text[..end].to_owned(),
                    });
                }
            }
            CoreContent::Text(text) => {
                left -= text.len();
                shown.push(content.to_dto());
            }
            CoreContent::Image { .. } => shown.push(content.to_dto()),
        }
    }
    (shown, cut)
}

#[cfg(test)]
mod tests {
    use copilot_core::{
        Approval, ChatId, Clock, FileId, MessageStatus, Role, SystemClock, ToolAlias, ToolCallPart,
        ToolStatus, UserId,
    };
    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn events_carry_their_seq_and_wire_names() {
        let message_id = MessageId::generate();
        let event = event(
            TurnEvent::TextDelta {
                message_id,
                index: 0,
                text: "Hi".to_owned(),
            },
            7,
        );
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            json!({
                "type": "text.delta",
                "seq": 7,
                "messageId": message_id.uuid(),
                "index": 0,
                "text": "Hi",
            })
        );
        assert_eq!(
            serde_json::to_value(super::event(TurnEvent::Done, 8)).unwrap(),
            json!({"type": "done", "seq": 8})
        );
    }

    #[test]
    fn tool_output_is_cut_for_display_on_a_character_boundary() {
        let long = "é".repeat(DISPLAY_MAX_BYTES);
        let event = event(
            TurnEvent::ToolFinished {
                message_id: MessageId::generate(),
                call_id: CallId::generate(),
                status: ToolStatus::Ok,
                output: vec![
                    CoreContent::Text(long),
                    CoreContent::Text("more".to_owned()),
                    CoreContent::Image {
                        file_id: FileId::generate(),
                    },
                ],
                truncated: false,
            },
            1,
        );
        let ChatStreamEvent::ToolFinished {
            output, truncated, ..
        } = event
        else {
            panic!("a tool.finished event");
        };
        assert!(truncated);
        assert_eq!(output.len(), 2, "the cut text and the image");
        let ResultContent::Text { text } = &output[0] else {
            panic!("text first");
        };
        assert_eq!(text.len(), DISPLAY_MAX_BYTES);
        assert!(matches!(output[1], ResultContent::Image { .. }));
    }

    #[test]
    fn messages_never_show_aliases_or_provider_ids() {
        let call = ToolCallPart {
            id: CallId::generate(),
            provider_call_id: Some("call_secret".to_owned()),
            server_id: None,
            tool_name: "list_schedules".to_owned(),
            alias: ToolAlias::restore("list_schedules".to_owned()),
            arguments_raw: "{oops".to_owned(),
            arguments: None,
            complete: true,
            approval: Approval::Auto,
            deny_reason: None,
        };
        let message = Message::user(
            UserId::new(Uuid::now_v7()),
            ChatId::generate(),
            vec![Part::ToolCall(call)],
            SystemClock.now(),
        );
        let body = serde_json::to_value(super::message(&message)).unwrap();
        assert_eq!(body["role"], "user");
        assert_eq!(body["status"], "complete");
        assert_eq!(
            body["parts"][0]["arguments"], "{oops",
            "raw text when not an object"
        );
        let text = body.to_string();
        assert!(!text.contains("call_secret"));
        assert!(!text.contains("\"alias\""));
        assert_eq!(MessageStatus::Complete, message.status());
        assert_eq!(Role::User, message.role());
    }
}
