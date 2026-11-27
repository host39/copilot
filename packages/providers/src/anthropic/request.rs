//! Messages API request bodies (`POST {base}/v1/messages`, streamed), per Adapter Rules in
//! `docs/design/copilot.md` and the Anthropic section of `docs/research/providers.md`.
//!
//! - `model`, `max_tokens` (required), `stream: true`, and the system text as the top-level
//!   `system`. Never `temperature`, `top_p`, or `top_k` (API version `2026-10-21` rejects them),
//!   never `tool_choice` (`auto` is the default; forced tool use is rejected by newer models).
//! - Tools are `{name, description, input_schema}` with an object schema.
//! - No assistant prefill: a request whose last message is an answer is refused (core never
//!   builds one; current models reject it).
//! - **Reasoning.** No level: no `thinking` for a model without `reasoning` (`can_reason`);
//!   a reasoning model gets `thinking: {type: "adaptive", display: "summarized"}` without an
//!   effort, so its default effort applies and its thinking shows. `off`:
//!   `thinking: {type: "disabled"}`. `low`, `medium`, `high`: adaptive thinking with summarized
//!   display and that `output_config.effort`. Never `budget_tokens`.
//! - User parts: text blocks, images as base64 `image` blocks (JPEG, PNG, GIF, WebP), PDFs as
//!   base64 `document` blocks titled with the file name. Audio is never sent (Claude models take
//!   none; core locks the flags).
//! - Assistant messages: a replayed one sends its stored content blocks verbatim (`thinking`
//!   with its signature, `redacted_thinking`, text, `tool_use`), as core's Replay Rules allow;
//!   any other is rebuilt as text and `tool_use` blocks. Empty text blocks are never sent.
//! - Tool results are `tool_result` blocks first in a user message (text and image content,
//!   `is_error` for failures); text parts after them (results lowered to text) follow as text
//!   blocks. Adjacent entries of one role are merged into one message.
//! - **Cache breakpoints** (`cache_breakpoints`): `cache_control: {type: "ephemeral"}` on the
//!   system block (it caches the tools with it), on the last block of the first user message
//!   (it starts with the file preamble and stays the same until the window moves), and on the
//!   last block of the request. Three of the four breakpoints the API allows.
//!
//! Anything the protocol cannot carry fails with `PROVIDER_UNSUPPORTED` before anything is
//! sent, so a request never goes out with something silently dropped.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use copilot_core::{
    DomainError, DomainResult, LlmMessage, LlmPart, LlmRequest, ReasoningLevel, Role, ToolContent,
    ToolSpec,
};
use serde_json::{Map, Value, json};

use crate::gateway::UNSUPPORTED;
use crate::schema::object_schema;

/// The image types the Messages API reads.
const IMAGE_TYPES: [&str; 4] = ["image/jpeg", "image/png", "image/gif", "image/webp"];

/// The request body for `request`.
pub(crate) fn body(request: &LlmRequest) -> DomainResult<Value> {
    let mut messages = Messages::default();
    let mut first_user = true;
    for item in &request.messages {
        match item.role {
            Role::User => {
                messages.push(USER, user_blocks(&item.parts)?);
                if first_user {
                    messages.anchor();
                    first_user = false;
                }
            }
            Role::Assistant => messages.push(ASSISTANT, assistant_blocks(item)?),
            Role::Tool => messages.push(USER, tool_blocks(item)?),
        }
    }
    // Current models reject a last assistant message (prefill); core never builds one.
    if messages
        .entries
        .last()
        .is_some_and(|(role, _)| *role == ASSISTANT)
    {
        return Err(unsupported("a request that ends with an answer"));
    }
    let caching = request.cache_breakpoints;
    if caching {
        messages.mark();
    }

    let mut body = Map::new();
    body.insert("model".to_owned(), Value::from(request.model.as_str()));
    body.insert(
        "max_tokens".to_owned(),
        Value::from(request.max_output_tokens),
    );
    body.insert("stream".to_owned(), Value::Bool(true));
    if !request.system.is_empty() {
        let system = if caching {
            json!([{ "type": "text", "text": request.system, "cache_control": ephemeral() }])
        } else {
            Value::from(request.system.as_str())
        };
        body.insert("system".to_owned(), system);
    }
    body.insert("messages".to_owned(), Value::Array(messages.finish()));
    if !request.tools.is_empty() {
        body.insert(
            "tools".to_owned(),
            Value::Array(request.tools.iter().map(tool_spec).collect()),
        );
    }
    match request.reasoning {
        // A reasoning model without a level thinks at its default effort; ask to see a summary
        // (current models omit their thinking text unless asked).
        None if request.can_reason => {
            body.insert(
                "thinking".to_owned(),
                json!({ "type": "adaptive", "display": "summarized" }),
            );
        }
        None => {}
        Some(ReasoningLevel::Off) => {
            body.insert("thinking".to_owned(), json!({ "type": "disabled" }));
        }
        Some(level) => {
            body.insert(
                "thinking".to_owned(),
                json!({ "type": "adaptive", "display": "summarized" }),
            );
            body.insert(
                "output_config".to_owned(),
                json!({ "effort": level.as_str() }),
            );
        }
    }
    Ok(Value::Object(body))
}

const USER: &str = "user";
const ASSISTANT: &str = "assistant";

/// The message list, merging adjacent entries of one role.
#[derive(Default)]
struct Messages {
    entries: Vec<(&'static str, Vec<Value>)>,
    /// The last block of the first user message: entry and block index.
    anchor: Option<(usize, usize)>,
}

impl Messages {
    fn push(&mut self, role: &'static str, blocks: Vec<Value>) {
        if blocks.is_empty() {
            return;
        }
        match self.entries.last_mut() {
            Some((open, content)) if *open == role => content.extend(blocks),
            _ => self.entries.push((role, blocks)),
        }
    }

    /// Remembers the last block so far as the end of the first user message.
    fn anchor(&mut self) {
        if let Some((USER, content)) = self.entries.last()
            && let Some(last) = content.len().checked_sub(1)
        {
            self.anchor = Some((self.entries.len() - 1, last));
        }
    }

    /// Marks the anchor and the last block of the request as cache breakpoints.
    fn mark(&mut self) {
        let last = self.entries.len().checked_sub(1).and_then(|entry| {
            let blocks = self.entries[entry].1.len();
            blocks.checked_sub(1).map(|block| (entry, block))
        });
        for (entry, block) in [self.anchor, last].into_iter().flatten() {
            if let Some(Value::Object(block)) = self.entries[entry].1.get_mut(block)
                && cacheable(block)
            {
                block.insert("cache_control".to_owned(), ephemeral());
            }
        }
    }

    fn finish(self) -> Vec<Value> {
        self.entries
            .into_iter()
            .map(|(role, content)| json!({ "role": role, "content": content }))
            .collect()
    }
}

/// Whether a block may carry `cache_control` (thinking blocks may not).
fn cacheable(block: &Map<String, Value>) -> bool {
    !matches!(
        block.get("type").and_then(Value::as_str),
        Some("thinking" | "redacted_thinking")
    )
}

fn ephemeral() -> Value {
    json!({ "type": "ephemeral" })
}

/// The blocks of a user message.
fn user_blocks(parts: &[LlmPart]) -> DomainResult<Vec<Value>> {
    let mut blocks = Vec::new();
    for part in parts {
        match part {
            LlmPart::Text(text) => blocks.extend(text_block(text)),
            LlmPart::Image { media_type, data } => {
                blocks.push(image(media_type, data).ok_or_else(|| unsupported("this image type"))?);
            }
            LlmPart::Pdf { name, data } => blocks.push(json!({
                "type": "document",
                "source": {
                    "type": "base64",
                    "media_type": "application/pdf",
                    "data": STANDARD.encode(data),
                },
                "title": name,
            })),
            LlmPart::Audio { .. } => return Err(unsupported("audio")),
            LlmPart::ToolCall { .. } | LlmPart::ToolResult { .. } => {
                return Err(unsupported("tool parts in a user message"));
            }
        }
    }
    Ok(blocks)
}

/// The blocks of an assistant message: its replayed content, else its text and tool calls.
fn assistant_blocks(message: &LlmMessage) -> DomainResult<Vec<Value>> {
    if let Some(blocks) = message.replay.as_ref().and_then(replayed) {
        return Ok(blocks);
    }
    let mut blocks = Vec::new();
    for part in &message.parts {
        match part {
            LlmPart::Text(text) => blocks.extend(text_block(text)),
            LlmPart::ToolCall {
                provider_call_id,
                alias,
                arguments,
                ..
            } => {
                let input = if arguments.is_object() {
                    arguments.clone()
                } else {
                    Value::Object(Map::new())
                };
                blocks.push(json!({
                    "type": "tool_use",
                    "id": provider_call_id,
                    "name": alias,
                    "input": input,
                }));
            }
            LlmPart::Image { .. } | LlmPart::Pdf { .. } | LlmPart::Audio { .. } => {
                return Err(unsupported("files in an answer"));
            }
            LlmPart::ToolResult { .. } => return Err(unsupported("tool results in an answer")),
        }
    }
    Ok(blocks)
}

/// The stored content blocks of a replayed answer, when they read as a non-empty list of typed
/// blocks. Anything else is ignored, so a stored state never stops a chat; the answer is then
/// rebuilt from its parts.
pub(crate) fn replayed(state: &Value) -> Option<Vec<Value>> {
    let blocks = state.as_array()?;
    let typed = blocks
        .iter()
        .all(|block| block.get("type").and_then(Value::as_str).is_some());
    (typed && !blocks.is_empty()).then(|| blocks.clone())
}

/// The blocks of a tool message: its `tool_result` blocks, then its text parts.
fn tool_blocks(message: &LlmMessage) -> DomainResult<Vec<Value>> {
    let mut results = Vec::new();
    let mut texts = Vec::new();
    for part in &message.parts {
        match part {
            LlmPart::ToolResult {
                provider_call_id,
                is_error,
                content,
                ..
            } => {
                let mut result = Map::new();
                result.insert("type".to_owned(), Value::from("tool_result"));
                result.insert(
                    "tool_use_id".to_owned(),
                    Value::from(provider_call_id.as_str()),
                );
                let content = result_content(content);
                if !content.is_empty() {
                    result.insert("content".to_owned(), Value::Array(content));
                }
                if *is_error {
                    result.insert("is_error".to_owned(), Value::Bool(true));
                }
                results.push(Value::Object(result));
            }
            LlmPart::Text(text) => texts.extend(text_block(text)),
            _ => return Err(unsupported("files in a tool result")),
        }
    }
    results.extend(texts);
    Ok(results)
}

/// The content of a `tool_result`: text and image blocks; audio and other binary content
/// become a notice, links `name: uri`.
fn result_content(content: &[ToolContent]) -> Vec<Value> {
    content
        .iter()
        .filter_map(|item| match item {
            ToolContent::Text(text) => text_block(text),
            ToolContent::Image { media_type, data } => image(media_type, data)
                .or_else(|| text_block("[image omitted: this model cannot read this image type]")),
            ToolContent::Audio { .. } => {
                text_block("[audio omitted: this model cannot read audio]")
            }
            ToolContent::Link { uri, name } => match name {
                Some(name) => text_block(&format!("{name}: {uri}")),
                None => text_block(uri),
            },
            ToolContent::Binary { .. } => {
                text_block("[binary content omitted: this model cannot read it]")
            }
        })
        .collect()
}

/// A tool definition with an object input schema.
fn tool_spec(spec: &ToolSpec) -> Value {
    let mut tool = Map::new();
    tool.insert("name".to_owned(), Value::from(spec.alias.as_str()));
    if !spec.description.trim().is_empty() {
        tool.insert(
            "description".to_owned(),
            Value::from(spec.description.as_str()),
        );
    }
    tool.insert("input_schema".to_owned(), object_schema(&spec.input_schema));
    Value::Object(tool)
}

/// A text block, or none for empty text (the API rejects empty text blocks).
fn text_block(text: &str) -> Option<Value> {
    (!text.is_empty()).then(|| json!({ "type": "text", "text": text }))
}

/// A base64 image block, or none for a type the API does not read.
fn image(media_type: &str, data: &[u8]) -> Option<Value> {
    IMAGE_TYPES.contains(&media_type).then(|| {
        json!({
            "type": "image",
            "source": { "type": "base64", "media_type": media_type, "data": STANDARD.encode(data) },
        })
    })
}

fn unsupported(what: &str) -> DomainError {
    DomainError::upstream(format!("This connection cannot send {what}."), UNSUPPORTED)
}

#[cfg(test)]
mod tests;
