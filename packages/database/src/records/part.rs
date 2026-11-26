//! `PartRecord`: one element of `messages.parts`.

use copilot_core::{
    CallId, FileId, Part, ResultContent, ToolAlias, ToolCallPart, ToolResultPart, ToolServerId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::records::{RecordError, VERSION, check, parse};

/// A message part as stored. Tool calls keep the provider's call id and the alias, which never
/// reach clients.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PartRecord {
    Text {
        v: u8,
        text: String,
    },
    File {
        v: u8,
        file_id: Uuid,
        name: String,
        media_type: String,
    },
    Audio {
        v: u8,
        file_id: Uuid,
        format: String,
        transcript: Option<String>,
        duration_ms: Option<u32>,
    },
    Reasoning {
        v: u8,
        text: String,
    },
    ToolCall {
        v: u8,
        /// Copilot's call id.
        id: Uuid,
        provider_call_id: Option<String>,
        /// `null` for a built-in tool.
        server_id: Option<Uuid>,
        tool_name: String,
        alias: String,
        arguments_raw: String,
        /// The parsed arguments when they are a JSON object.
        arguments: Option<Value>,
        complete: bool,
        /// `auto`, `pending`, `approved`, or `denied`.
        approval: String,
        deny_reason: Option<String>,
    },
    ToolResult {
        v: u8,
        call_id: Uuid,
        /// `ok`, `error`, `denied`, `cancelled`, or `timeout`.
        status: String,
        is_error: bool,
        content: Vec<ContentRecord>,
        truncated: bool,
    },
}

/// One item of a tool result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentRecord {
    Text {
        text: String,
    },
    /// An image result, saved as a chat file.
    Image {
        file_id: Uuid,
    },
}

impl From<&Part> for PartRecord {
    fn from(part: &Part) -> Self {
        let v = VERSION;
        match part {
            Part::Text { text } => Self::Text {
                v,
                text: text.clone(),
            },
            Part::File {
                file_id,
                name,
                media_type,
            } => Self::File {
                v,
                file_id: file_id.uuid(),
                name: name.clone(),
                media_type: media_type.clone(),
            },
            Part::Audio {
                file_id,
                format,
                transcript,
                duration_ms,
            } => Self::Audio {
                v,
                file_id: file_id.uuid(),
                format: format.clone(),
                transcript: transcript.clone(),
                duration_ms: *duration_ms,
            },
            Part::Reasoning { text } => Self::Reasoning {
                v,
                text: text.clone(),
            },
            Part::ToolCall(call) => Self::ToolCall {
                v,
                id: call.id.uuid(),
                provider_call_id: call.provider_call_id.clone(),
                server_id: call.server_id.map(ToolServerId::uuid),
                tool_name: call.tool_name.clone(),
                alias: call.alias.as_str().to_owned(),
                arguments_raw: call.arguments_raw.clone(),
                arguments: call.arguments.clone(),
                complete: call.complete,
                approval: call.approval.as_str().to_owned(),
                deny_reason: call.deny_reason.clone(),
            },
            Part::ToolResult(result) => Self::ToolResult {
                v,
                call_id: result.call_id.uuid(),
                status: result.status.as_str().to_owned(),
                is_error: result.is_error,
                content: result.content.iter().map(ContentRecord::from).collect(),
                truncated: result.truncated,
            },
        }
    }
}

impl TryFrom<PartRecord> for Part {
    type Error = RecordError;

    fn try_from(record: PartRecord) -> Result<Self, Self::Error> {
        Ok(match record {
            PartRecord::Text { v, text } => {
                check(v)?;
                Self::Text { text }
            }
            PartRecord::File {
                v,
                file_id,
                name,
                media_type,
            } => {
                check(v)?;
                Self::File {
                    file_id: FileId::new(file_id),
                    name,
                    media_type,
                }
            }
            PartRecord::Audio {
                v,
                file_id,
                format,
                transcript,
                duration_ms,
            } => {
                check(v)?;
                Self::Audio {
                    file_id: FileId::new(file_id),
                    format,
                    transcript,
                    duration_ms,
                }
            }
            PartRecord::Reasoning { v, text } => {
                check(v)?;
                Self::Reasoning { text }
            }
            PartRecord::ToolCall {
                v,
                id,
                provider_call_id,
                server_id,
                tool_name,
                alias,
                arguments_raw,
                arguments,
                complete,
                approval,
                deny_reason,
            } => {
                check(v)?;
                Self::ToolCall(ToolCallPart {
                    id: CallId::new(id),
                    provider_call_id,
                    server_id: server_id.map(ToolServerId::new),
                    tool_name,
                    alias: ToolAlias::restore(alias),
                    arguments_raw,
                    arguments,
                    complete,
                    approval: parse(&approval)?,
                    deny_reason,
                })
            }
            PartRecord::ToolResult {
                v,
                call_id,
                status,
                is_error,
                content,
                truncated,
            } => {
                check(v)?;
                Self::ToolResult(ToolResultPart {
                    call_id: CallId::new(call_id),
                    status: parse(&status)?,
                    is_error,
                    content: content.into_iter().map(ResultContent::from).collect(),
                    truncated,
                })
            }
        })
    }
}

impl From<&ResultContent> for ContentRecord {
    fn from(content: &ResultContent) -> Self {
        match content {
            ResultContent::Text(text) => Self::Text { text: text.clone() },
            ResultContent::Image { file_id } => Self::Image {
                file_id: file_id.uuid(),
            },
        }
    }
}

impl From<ContentRecord> for ResultContent {
    fn from(record: ContentRecord) -> Self {
        match record {
            ContentRecord::Text { text } => Self::Text(text),
            ContentRecord::Image { file_id } => Self::Image {
                file_id: FileId::new(file_id),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use copilot_core::{Approval, ToolStatus};
    use serde_json::json;

    use super::*;

    fn every_part() -> Vec<Part> {
        let call = CallId::generate();
        vec![
            Part::Text {
                text: "Hello".to_owned(),
            },
            Part::File {
                file_id: FileId::generate(),
                name: "notes.txt".to_owned(),
                media_type: "text/plain".to_owned(),
            },
            Part::Audio {
                file_id: FileId::generate(),
                format: "wav".to_owned(),
                transcript: Some("Hi".to_owned()),
                duration_ms: Some(1_200),
            },
            Part::Reasoning {
                text: "Thinking".to_owned(),
            },
            Part::ToolCall(ToolCallPart {
                id: call,
                provider_call_id: Some("call_1".to_owned()),
                server_id: Some(ToolServerId::generate()),
                tool_name: "search".to_owned(),
                alias: ToolAlias::restore("docs__search".to_owned()),
                arguments_raw: "{\"q\":\"rust\"}".to_owned(),
                arguments: Some(json!({"q": "rust"})),
                complete: true,
                approval: Approval::Denied,
                deny_reason: Some("Not now.".to_owned()),
            }),
            Part::ToolResult(ToolResultPart {
                call_id: call,
                status: ToolStatus::Timeout,
                is_error: true,
                content: vec![
                    ResultContent::Text("Too slow.".to_owned()),
                    ResultContent::Image {
                        file_id: FileId::generate(),
                    },
                ],
                truncated: true,
            }),
        ]
    }

    #[test]
    fn every_part_round_trips_through_json() {
        for part in every_part() {
            let json = serde_json::to_value(PartRecord::from(&part)).unwrap();
            let record: PartRecord = serde_json::from_value(json).unwrap();
            assert_eq!(Part::try_from(record).unwrap(), part);
        }
    }

    #[test]
    fn stored_shapes_are_tagged_and_versioned() {
        let text = serde_json::to_value(PartRecord::from(&Part::Text {
            text: "Hi".to_owned(),
        }))
        .unwrap();
        assert_eq!(text, json!({"type": "text", "v": 1, "text": "Hi"}));

        let parts = every_part();
        let call = serde_json::to_value(PartRecord::from(&parts[4])).unwrap();
        assert_eq!(call["type"], "tool_call");
        assert_eq!(call["approval"], "denied");
        assert_eq!(call["alias"], "docs__search");
        let result = serde_json::to_value(PartRecord::from(&parts[5])).unwrap();
        assert_eq!(result["type"], "tool_result");
        assert_eq!(result["status"], "timeout");
        assert_eq!(
            result["content"][0],
            json!({"type": "text", "text": "Too slow."})
        );
        assert_eq!(result["content"][1]["type"], "image");
    }

    #[test]
    fn unknown_versions_and_values_do_not_read() {
        let newer: PartRecord =
            serde_json::from_value(json!({"type": "text", "v": 2, "text": "Hi"})).unwrap();
        assert!(matches!(
            Part::try_from(newer),
            Err(RecordError::Version(2))
        ));
        let mut call = serde_json::to_value(PartRecord::from(&every_part()[4])).unwrap();
        call["approval"] = json!("maybe");
        let record: PartRecord = serde_json::from_value(call).unwrap();
        assert!(matches!(
            Part::try_from(record),
            Err(RecordError::Value(value)) if value == "maybe"
        ));
    }
}
