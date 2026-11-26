//! Tool call results: the built-in schedule tools, and MCP output converted to a stored result
//! (`docs/design/copilot.md`, Tool Execution and Approvals, Results).

use bytes::Bytes;
use serde_json::{Map, Value, json};

use crate::domains::chat::{ResultContent, ToolCallPart, ToolResultPart, ToolStatus};
use crate::domains::conversation::approval::UNKNOWN_TOOL;
use crate::domains::file::{FileOwner, FileService, StoredFile};
use crate::domains::schedule::{
    Schedule, SchedulePrompt, ScheduleService, ScheduleTool, ToolOrigin, cancel_target,
};
use crate::ports::tool_host::{ToolContent, ToolOutput};
use crate::shared::actor::Actor;
use crate::shared::constants::MCP_RESULT_MAX_BYTES;
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::ChatId;

/// The text of a failure the user should not see the details of.
pub(crate) const FAILED: &str = "Something went wrong while answering. Try again.";

/// A human-readable message for `error`; internal errors read as [`FAILED`].
pub(crate) fn human(error: &DomainError) -> String {
    match error {
        DomainError::Internal { .. } => FAILED.to_owned(),
        other => other.to_string(),
    }
}

/// An `error` result with one line of text.
pub(crate) fn failed(call: &ToolCallPart, text: &str) -> ToolResultPart {
    ToolResultPart {
        call_id: call.id,
        status: ToolStatus::Error,
        is_error: true,
        content: vec![ResultContent::Text(text.to_owned())],
        truncated: false,
    }
}

/// Runs a built-in schedule tool for the calling chat. The result is JSON for the model.
pub(crate) async fn builtin(
    schedules: &ScheduleService,
    actor: &Actor,
    origin: &ToolOrigin,
    call: &ToolCallPart,
) -> ToolResultPart {
    let arguments = call
        .arguments
        .clone()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let outcome: DomainResult<Value> = match ScheduleTool::find(&call.tool_name) {
        Some(ScheduleTool::Prompt) => match SchedulePrompt::parse(&arguments) {
            Ok(input) => schedules
                .schedule_prompt(actor, origin, input)
                .await
                .map(|schedule| json!({ "created": summary(&schedule) })),
            Err(error) => Err(error),
        },
        Some(ScheduleTool::List) => schedules
            .list_schedules(actor, origin)
            .await
            .map(|list| json!({ "schedules": list.iter().map(summary).collect::<Vec<_>>() })),
        Some(ScheduleTool::Cancel) => match cancel_target(&arguments) {
            Ok(id) => schedules
                .cancel_schedule(actor, origin, id)
                .await
                .map(|schedule| {
                    json!({ "deleted": { "id": schedule.id().to_string(), "name": schedule.name() } })
                }),
            Err(error) => Err(error),
        },
        None => Err(DomainError::validation(UNKNOWN_TOOL)),
    };
    match outcome {
        Ok(value) => ToolResultPart {
            call_id: call.id,
            status: ToolStatus::Ok,
            is_error: false,
            content: vec![ResultContent::Text(value.to_string())],
            truncated: false,
        },
        Err(error) => failed(call, &human(&error)),
    }
}

fn summary(schedule: &Schedule) -> Value {
    json!({
        "id": schedule.id().to_string(),
        "name": schedule.name(),
        "cron": schedule.rule().expression(),
        "timezone": schedule.rule().timezone(),
        "description": schedule.rule().describe(),
        "prompt": schedule.prompt(),
        "enabled": schedule.enabled(),
        "next_run_at": schedule.next_run_at().map(|at| at.to_rfc3339()),
    })
}

/// A stored result from what a tool returned: text cut to `MCP_RESULT_MAX_BYTES` in total with
/// a `[truncated N bytes]` note, images saved as chat files (the upload rules apply), audio and
/// other binaries as a one-line notice, and resource links as their name and URI. Returns the
/// saved images too.
pub(crate) async fn convert(
    files: &FileService,
    actor: &Actor,
    chat: ChatId,
    call: &ToolCallPart,
    output: ToolOutput,
) -> (ToolResultPart, Vec<StoredFile>) {
    let mut texts = Texts::new(MCP_RESULT_MAX_BYTES);
    let mut saved = Vec::new();
    for item in output.content {
        match item {
            ToolContent::Text(text) => texts.push(text),
            ToolContent::Image { media_type, data } => {
                match save(files, actor, chat, &media_type, data).await {
                    Ok(file) => {
                        texts
                            .content
                            .push(ResultContent::Image { file_id: file.id() });
                        saved.push(file);
                    }
                    Err(reason) => texts.push(format!("[image omitted: {reason}]")),
                }
            }
            ToolContent::Audio { media_type, .. } => {
                texts.push(format!("[audio omitted: {media_type}]"));
            }
            ToolContent::Binary { media_type } => texts.push(match media_type {
                Some(media_type) => format!("[binary content omitted: {media_type}]"),
                None => "[binary content omitted]".to_owned(),
            }),
            ToolContent::Link { uri, name } => texts.push(match name {
                Some(name) if !name.is_empty() => format!("{name}: {uri}"),
                _ => uri,
            }),
        }
    }
    let truncated = texts.cut > 0;
    if truncated {
        let note = format!("[truncated {} bytes]", texts.cut);
        texts.content.push(ResultContent::Text(note));
    }
    let result = ToolResultPart {
        call_id: call.id,
        status: if output.is_error {
            ToolStatus::Error
        } else {
            ToolStatus::Ok
        },
        is_error: output.is_error,
        content: texts.content,
        truncated,
    };
    (result, saved)
}

/// Saves a result image as an attachment of the chat.
async fn save(
    files: &FileService,
    actor: &Actor,
    chat: ChatId,
    media_type: &str,
    data: Bytes,
) -> Result<StoredFile, String> {
    let extension = match media_type.trim().to_ascii_lowercase().as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => return Err("this image type is not supported".to_owned()),
    };
    files
        .upload(
            actor,
            FileOwner::Chat(chat),
            &format!("tool-image.{extension}"),
            data,
        )
        .await
        .map_err(|error| human(&error))
}

/// Result text under a byte budget.
struct Texts {
    content: Vec<ResultContent>,
    left: usize,
    /// Bytes cut so far.
    cut: usize,
}

impl Texts {
    fn new(budget: usize) -> Self {
        Self {
            content: Vec::new(),
            left: budget,
            cut: 0,
        }
    }

    fn push(&mut self, text: String) {
        if text.len() <= self.left {
            self.left -= text.len();
            self.content.push(ResultContent::Text(text));
            return;
        }
        let mut end = self.left;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.cut += text.len() - end;
        self.left = 0;
        if end > 0 {
            self.content
                .push(ResultContent::Text(text[..end].to_owned()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_text_at_a_character_boundary_and_counts_the_rest() {
        let mut texts = Texts::new(5);
        texts.push("abc".to_owned());
        texts.push("déf".to_owned());
        texts.push("ghi".to_owned());
        assert_eq!(
            texts.content,
            [
                ResultContent::Text("abc".to_owned()),
                ResultContent::Text("d".to_owned()),
            ]
        );
        assert_eq!(texts.cut, 3 + 3);
    }

    #[test]
    fn internal_errors_stay_generic() {
        assert_eq!(
            human(&DomainError::internal(
                "database.messages.update.failed",
                "boom"
            )),
            FAILED
        );
        assert_eq!(
            human(&DomainError::validation("Choose a model.")),
            "Choose a model."
        );
    }
}
