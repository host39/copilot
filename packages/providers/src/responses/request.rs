//! Responses API request bodies (`POST {base}/responses`, streamed, stateless), per
//! `docs/research/providers.md` (OpenAI Responses) and the Adapter Rules in
//! `docs/design/copilot.md`.
//!
//! - `store: false`: nothing is kept on the provider's side, so every request carries the whole
//!   history, and `include: ["reasoning.encrypted_content"]` asks for the encrypted reasoning a
//!   later request replays (OpenAI returns it by default in stateless mode; compatible servers
//!   may still need the include). Sent with the reasoning settings, only for models with the
//!   `reasoning` capability.
//! - The system text is `instructions`; the output limit `max_output_tokens` (at least 16, the
//!   API's minimum).
//! - `reasoning`: `effort` (`none`, `low`, `medium`, `high`) when core picked a level, and
//!   `summary: "auto"` so the answer shows a summary of its reasoning, unless reasoning is off.
//! - Tools are `{type: "function", name, description, parameters, strict: false}` with
//!   `tool_choice: "auto"`. `strict` is sent explicitly: Responses otherwise tries strict mode,
//!   which would make every optional argument of an MCP tool required.
//! - User parts: `input_text`, images as `input_image` with a base64 data URL, PDFs as
//!   `input_file` with a `data:application/pdf;base64,` URL. Audio fails with
//!   `PROVIDER_UNSUPPORTED`: the Responses API takes no audio yet (core locks `audio_in` on such
//!   connections, so it sends a notice instead). Adjacent user messages are merged.
//! - An assistant message whose replay state is a Responses state goes out as its stored output
//!   items (encrypted reasoning, messages with their `phase`, function calls), as long as they
//!   hold exactly the message's calls; otherwise it is rebuilt from its parts: its text as an
//!   assistant message and each call as a `function_call` item without an `id`.
//! - Each tool result is a `function_call_output`: text, or a list of `input_text` and
//!   `input_image` items when the result carries images. Text parts after the results (results
//!   lowered to text) go out as user text.
//!
//! Anything the protocol cannot carry fails with `PROVIDER_UNSUPPORTED` before anything is
//! sent.

use std::collections::BTreeSet;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use copilot_core::{
    DomainError, DomainResult, LlmMessage, LlmPart, LlmRequest, ReasoningLevel, Role, ToolContent,
    ToolSpec,
};
use serde_json::{Map, Value, json};

use crate::gateway::UNSUPPORTED;
use crate::responses::replay;
use crate::schema::object_schema;

/// The smallest `max_output_tokens` the API takes.
const MIN_OUTPUT_TOKENS: u32 = 16;
/// What `include` asks for: reasoning a stateless request can send back.
const ENCRYPTED_REASONING: &str = "reasoning.encrypted_content";

/// The request body for `request`.
pub(crate) fn body(request: &LlmRequest) -> DomainResult<Value> {
    if request.audio_out.is_some() {
        return Err(unsupported("spoken replies"));
    }
    let mut input = Input::default();
    for message in &request.messages {
        match message.role {
            Role::User => input.user(user_parts(&message.parts)?),
            Role::Assistant => assistant(message, &mut input)?,
            Role::Tool => tool(message, &mut input)?,
        }
    }

    let mut body = Map::new();
    body.insert("model".to_owned(), Value::from(request.model.as_str()));
    body.insert("stream".to_owned(), Value::Bool(true));
    body.insert("store".to_owned(), Value::Bool(false));
    if !request.system.is_empty() {
        body.insert(
            "instructions".to_owned(),
            Value::from(request.system.as_str()),
        );
    }
    body.insert("input".to_owned(), Value::Array(input.finish()));
    body.insert(
        "max_output_tokens".to_owned(),
        Value::from(request.max_output_tokens.max(MIN_OUTPUT_TOKENS)),
    );
    if !request.tools.is_empty() {
        body.insert(
            "tools".to_owned(),
            Value::Array(request.tools.iter().map(tool_spec).collect()),
        );
        body.insert("tool_choice".to_owned(), Value::from("auto"));
    }
    if request.can_reason || request.reasoning.is_some() {
        let mut reasoning = Map::new();
        if let Some(level) = request.reasoning {
            reasoning.insert("effort".to_owned(), Value::from(effort(level)));
        }
        if request.reasoning != Some(ReasoningLevel::Off) {
            reasoning.insert("summary".to_owned(), Value::from("auto"));
            body.insert("include".to_owned(), json!([ENCRYPTED_REASONING]));
        }
        body.insert("reasoning".to_owned(), Value::Object(reasoning));
    }
    Ok(Value::Object(body))
}

/// The input items, merging adjacent user messages.
#[derive(Default)]
struct Input {
    entries: Vec<Entry>,
}

enum Entry {
    /// The content parts of one user message.
    User(Vec<Value>),
    Item(Value),
}

impl Input {
    fn push(&mut self, item: Value) {
        self.entries.push(Entry::Item(item));
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

    fn finish(self) -> Vec<Value> {
        self.entries
            .into_iter()
            .map(|entry| match entry {
                Entry::Item(item) => item,
                Entry::User(parts) => {
                    json!({ "type": "message", "role": "user", "content": parts })
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
            LlmPart::Image { media_type, data } => Ok(image_part(media_type, data)),
            LlmPart::Pdf { name, data } => Ok(json!({
                "type": "input_file",
                "filename": name,
                "file_data": data_url("application/pdf", data),
            })),
            LlmPart::Audio { .. } => Err(unsupported("audio")),
            LlmPart::ToolCall { .. } | LlmPart::ToolResult { .. } => {
                Err(unsupported("tool parts in a user message"))
            }
        })
        .collect()
}

/// An assistant message: its stored output items when they replay, else its text and calls.
fn assistant(message: &LlmMessage, input: &mut Input) -> DomainResult<()> {
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
            } => calls.push(json!({
                "type": "function_call",
                "call_id": provider_call_id,
                "name": alias,
                "arguments": arguments.to_string(),
            })),
            LlmPart::Image { .. } | LlmPart::Pdf { .. } | LlmPart::Audio { .. } => {
                return Err(unsupported("files in an answer"));
            }
            LlmPart::ToolResult { .. } => return Err(unsupported("tool results in an answer")),
        }
    }
    if let Some(items) = message.replay.as_ref().and_then(replay::items)
        && same_calls(&items, &calls)
    {
        for item in items {
            input.push(item);
        }
        return Ok(());
    }
    let text = texts.join("\n\n");
    if !text.is_empty() {
        input.push(json!({ "type": "message", "role": "assistant", "content": text }));
    }
    for call in calls {
        input.push(call);
    }
    Ok(())
}

/// Whether the stored items hold exactly the calls of the message: every result that follows
/// must answer a call the request carries.
fn same_calls(items: &[Value], calls: &[Value]) -> bool {
    let stored: BTreeSet<&str> = items.iter().filter_map(replay::call_id).collect();
    let expected: BTreeSet<&str> = calls.iter().filter_map(replay::call_id).collect();
    stored == expected
}

/// A tool message: one `function_call_output` per result, then its text parts as user text.
fn tool(message: &LlmMessage, input: &mut Input) -> DomainResult<()> {
    let mut texts = Vec::new();
    for part in &message.parts {
        match part {
            LlmPart::ToolResult {
                provider_call_id,
                content,
                ..
            } => input.push(json!({
                "type": "function_call_output",
                "call_id": provider_call_id,
                "output": output(content),
            })),
            LlmPart::Text(text) => texts.push(text_part(text)),
            _ => return Err(unsupported("files in a tool result")),
        }
    }
    input.user(texts);
    Ok(())
}

/// A result's output: plain text, or `input_text` and `input_image` items when it carries an
/// image.
fn output(content: &[ToolContent]) -> Value {
    let images = content
        .iter()
        .any(|item| matches!(item, ToolContent::Image { .. }));
    if !images {
        let text: Vec<String> = content.iter().map(result_text).collect();
        return Value::from(text.join("\n"));
    }
    Value::Array(
        content
            .iter()
            .map(|item| match item {
                ToolContent::Image { media_type, data } => image_part(media_type, data),
                other => text_part(&result_text(other)),
            })
            .collect(),
    )
}

/// One result item as text; images are sent as `input_image` and never reach here.
fn result_text(item: &ToolContent) -> String {
    match item {
        ToolContent::Text(text) => text.clone(),
        ToolContent::Image { .. } => "[image]".to_owned(),
        ToolContent::Audio { .. } => {
            "[audio omitted: tool results reach this model as text and images only]".to_owned()
        }
        ToolContent::Link { uri, name } => match name {
            Some(name) => format!("{name}: {uri}"),
            None => uri.clone(),
        },
        ToolContent::Binary { .. } => {
            "[binary content omitted: tool results reach this model as text and images only]"
                .to_owned()
        }
    }
}

/// A function tool. The parameters are always an object schema.
fn tool_spec(spec: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "name": spec.alias.as_str(),
        "description": spec.description,
        "parameters": object_schema(&spec.input_schema),
        "strict": false,
    })
}

fn text_part(text: &str) -> Value {
    json!({ "type": "input_text", "text": text })
}

fn image_part(media_type: &str, data: &[u8]) -> Value {
    json!({ "type": "input_image", "image_url": data_url(media_type, data) })
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
    DomainError::upstream(
        format!("This connection cannot send {what} through the Responses API."),
        UNSUPPORTED,
    )
}

#[cfg(test)]
mod tests;
