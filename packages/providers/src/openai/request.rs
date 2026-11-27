//! Chat Completions request bodies (`POST {base}/chat/completions`, streamed), per dialect
//! (`docs/design/copilot.md`, Adapter Rules).
//!
//! - The system text goes under the dialect's role, the output limit under its field, and
//!   `stream_options` where the dialect reports usage that way.
//! - Tools are `{type: "function", function: {name, description, parameters, strict: false}}`,
//!   with `tool_choice: "auto"` except on Ollama.
//! - User parts: text, images as base64 data URLs, PDFs as `file` parts with a
//!   `data:application/pdf;base64,` URL, audio as `input_audio` (`wav` or `mp3`). Core sends a
//!   binary part only when the model has the capability.
//! - Assistant messages carry their text and `tool_calls`; a replayed one also carries its
//!   reasoning under the key it came under (OpenRouter: its `reasoning_details` instead, when it
//!   has them) and, on Gemini, each call's extra fields (thought signatures).
//! - Tool results are one `role: tool` message each, text only (an image becomes a notice);
//!   text parts after them (results lowered to text) go out as user text. Adjacent user
//!   messages are merged, for chat templates that need alternating roles.
//!
//! Anything the protocol cannot carry fails with `PROVIDER_UNSUPPORTED` before anything is
//! sent, so a request never goes out with something silently dropped.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use copilot_core::{
    Dialect, DomainError, DomainResult, LlmMessage, LlmPart, LlmRequest, ModelTarget,
    ReasoningLevel, Role, ToolContent, ToolSpec,
};
use serde_json::{Map, Value, json};

use crate::gateway::UNSUPPORTED;
use crate::openai::calls::CALL_FIELDS;
use crate::openai::replay::{REASONING, ReplayState};
use crate::schema::object_schema;

/// The request body for `request` in the target's dialect.
pub(crate) fn body(target: &ModelTarget, request: &LlmRequest) -> DomainResult<Value> {
    let dialect = target.dialect;
    let mut messages = Messages::default();
    if !request.system.is_empty() {
        let role = dialect.system_role().unwrap_or("system");
        messages.push(json!({ "role": role, "content": request.system }));
    }
    for item in &request.messages {
        match item.role {
            Role::User => messages.user(user_parts(&item.parts)?),
            Role::Assistant => messages.push(assistant(dialect, item)?),
            Role::Tool => tool(item, &mut messages)?,
        }
    }

    let mut body = Map::new();
    body.insert("model".to_owned(), Value::from(request.model.as_str()));
    body.insert("stream".to_owned(), Value::Bool(true));
    body.insert("messages".to_owned(), Value::Array(messages.finish()));
    body.insert(
        dialect.token_field().to_owned(),
        Value::from(request.max_output_tokens),
    );
    if dialect.stream_usage() {
        body.insert(
            "stream_options".to_owned(),
            json!({ "include_usage": true }),
        );
    }
    if !request.tools.is_empty() {
        body.insert(
            "tools".to_owned(),
            Value::Array(request.tools.iter().map(tool_spec).collect()),
        );
        if dialect.sends_tool_choice() {
            body.insert("tool_choice".to_owned(), Value::from("auto"));
        }
    }
    if let Some(level) = request.reasoning {
        if dialect == Dialect::OpenRouter {
            // OpenRouter's unified parameter; it maps the effort per model.
            body.insert("reasoning".to_owned(), json!({ "effort": effort(level) }));
        } else {
            body.insert("reasoning_effort".to_owned(), Value::from(effort(level)));
        }
    }
    if let Some(audio) = &request.audio_out {
        body.insert("modalities".to_owned(), json!(["text", "audio"]));
        body.insert(
            "audio".to_owned(),
            json!({ "voice": audio.voice, "format": audio.format }),
        );
    }
    Ok(Value::Object(body))
}

/// The message list, merging adjacent user messages.
#[derive(Default)]
struct Messages {
    entries: Vec<Entry>,
}

enum Entry {
    /// The content parts of one user message.
    User(Vec<Value>),
    Other(Value),
}

impl Messages {
    fn push(&mut self, message: Value) {
        self.entries.push(Entry::Other(message));
    }

    fn user(&mut self, parts: Vec<Value>) {
        if parts.is_empty() {
            return;
        }
        if let Some(Entry::User(open)) = self.entries.last_mut() {
            open.extend(parts);
        } else {
            self.entries.push(Entry::User(parts));
        }
    }

    /// A user message of one text part sends a string, any other an array of parts.
    fn finish(self) -> Vec<Value> {
        self.entries
            .into_iter()
            .map(|entry| match entry {
                Entry::Other(message) => message,
                Entry::User(mut parts) => {
                    let content = match parts.as_mut_slice() {
                        [part] if part["type"] == "text" => part["text"].take(),
                        _ => Value::Array(parts),
                    };
                    json!({ "role": "user", "content": content })
                }
            })
            .collect()
    }
}

/// The content parts of a user message.
fn user_parts(parts: &[LlmPart]) -> DomainResult<Vec<Value>> {
    parts
        .iter()
        .map(|part| match part {
            LlmPart::Text(text) => Ok(text_part(text)),
            LlmPart::Image { media_type, data } => Ok(json!({
                "type": "image_url",
                "image_url": { "url": data_url(media_type, data) },
            })),
            LlmPart::Pdf { name, data } => Ok(json!({
                "type": "file",
                "file": { "filename": name, "file_data": data_url("application/pdf", data) },
            })),
            LlmPart::Audio { format, data } => {
                if !matches!(format.as_str(), "wav" | "mp3") {
                    return Err(unsupported("this audio format"));
                }
                Ok(json!({
                    "type": "input_audio",
                    "input_audio": { "data": STANDARD.encode(data), "format": format },
                }))
            }
            LlmPart::ToolCall { .. } | LlmPart::ToolResult { .. } => {
                Err(unsupported("tool parts in a user message"))
            }
        })
        .collect()
}

/// An assistant message: its text joined by blank lines, its tool calls, and what its replay
/// state adds.
fn assistant(dialect: Dialect, message: &LlmMessage) -> DomainResult<Value> {
    let replay = message
        .replay
        .as_ref()
        .map(ReplayState::read)
        .unwrap_or_default();
    let mut texts = Vec::new();
    let mut calls = Vec::new();
    for part in &message.parts {
        match part {
            LlmPart::Text(text) => texts.push(text.as_str()),
            LlmPart::ToolCall {
                provider_call_id,
                alias,
                arguments,
                ..
            } => {
                let mut call = Map::new();
                if dialect.keeps_tool_call_extra()
                    && let Some(Value::Object(extra)) = replay.extra.get(provider_call_id)
                {
                    for (key, value) in extra {
                        if !CALL_FIELDS.contains(&key.as_str()) {
                            call.insert(key.clone(), value.clone());
                        }
                    }
                }
                call.insert("id".to_owned(), Value::from(provider_call_id.as_str()));
                call.insert("type".to_owned(), Value::from("function"));
                call.insert(
                    "function".to_owned(),
                    json!({ "name": alias, "arguments": arguments.to_string() }),
                );
                calls.push(Value::Object(call));
            }
            LlmPart::Image { .. } | LlmPart::Pdf { .. } | LlmPart::Audio { .. } => {
                return Err(unsupported("files in an answer"));
            }
            LlmPart::ToolResult { .. } => return Err(unsupported("tool results in an answer")),
        }
    }

    let mut out = Map::new();
    out.insert("role".to_owned(), Value::from("assistant"));
    let text = texts.join("\n\n");
    // Chat Completions wants `content: null` next to tool calls that have no text.
    let content = if text.is_empty() && !calls.is_empty() {
        Value::Null
    } else {
        Value::from(text)
    };
    out.insert("content".to_owned(), content);
    if !calls.is_empty() {
        out.insert("tool_calls".to_owned(), Value::Array(calls));
    }
    if dialect.replays_reasoning_details() && !replay.details.is_empty() {
        out.insert(
            "reasoning_details".to_owned(),
            Value::Array(replay.details.clone()),
        );
    } else if !replay.reasoning.is_empty() {
        out.insert(
            replay.key.unwrap_or(REASONING).to_owned(),
            Value::from(replay.reasoning.as_str()),
        );
    }
    Ok(Value::Object(out))
}

/// A tool message: one `role: tool` message per result, then its text parts as user text.
fn tool(message: &LlmMessage, messages: &mut Messages) -> DomainResult<()> {
    let mut texts = Vec::new();
    for part in &message.parts {
        match part {
            LlmPart::ToolResult {
                provider_call_id,
                content,
                ..
            } => messages.push(json!({
                "role": "tool",
                "tool_call_id": provider_call_id,
                "content": result_text(content),
            })),
            LlmPart::Text(text) => texts.push(text_part(text)),
            _ => return Err(unsupported("files in a tool result")),
        }
    }
    messages.user(texts);
    Ok(())
}

/// A result as text: Chat Completions tool messages carry text only.
fn result_text(content: &[ToolContent]) -> String {
    content
        .iter()
        .map(|item| match item {
            ToolContent::Text(text) => text.clone(),
            ToolContent::Image { .. } => {
                "[image omitted: tool results reach this model as text only]".to_owned()
            }
            ToolContent::Audio { .. } => {
                "[audio omitted: tool results reach this model as text only]".to_owned()
            }
            ToolContent::Link { uri, name } => match name {
                Some(name) => format!("{name}: {uri}"),
                None => uri.clone(),
            },
            ToolContent::Binary { .. } => {
                "[binary content omitted: tool results reach this model as text only]".to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A tool definition. The parameters are always an object schema: Chat Completions servers
/// reject anything else.
fn tool_spec(spec: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": spec.alias.as_str(),
            "description": spec.description,
            "parameters": object_schema(&spec.input_schema),
            "strict": false,
        },
    })
}

fn text_part(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

fn data_url(media_type: &str, data: &[u8]) -> String {
    format!("data:{media_type};base64,{}", STANDARD.encode(data))
}

/// The effort value: `none` turns reasoning off.
fn effort(level: ReasoningLevel) -> &'static str {
    match level {
        ReasoningLevel::Off => "none",
        ReasoningLevel::Low => "low",
        ReasoningLevel::Medium => "medium",
        ReasoningLevel::High => "high",
    }
}

fn unsupported(what: &str) -> DomainError {
    DomainError::upstream(format!("This connection cannot send {what}."), UNSUPPORTED)
}

#[cfg(test)]
mod tests;
