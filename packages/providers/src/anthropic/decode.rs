//! Messages API stream events to `LlmEvent`s (`docs/research/providers.md`, Anthropic,
//! Streaming and Thinking).
//!
//! Every event is named, and its data repeats the name in `type`: `message_start`, then per
//! content block `content_block_start`, `content_block_delta`s, and `content_block_stop`, then
//! `message_delta` (stop reason and cumulative usage) and `message_stop`. `ping` and unknown
//! events are skipped.
//!
//! - **Parts.** Text and thinking each continue their part while blocks of their kind follow one
//!   another; any other block ends the run, and a tool call is a part of its own. A part starts
//!   at its first non-empty delta, so thinking with omitted display (empty text) shows nothing.
//!   Indexes count up from 0 in arrival order.
//! - **Tool calls** start at `content_block_start` (`tool_use` id and name), take their
//!   arguments from `input_json_delta` fragments (or the start block's `input` when none come),
//!   and end at `content_block_stop`.
//! - **Replay state** is the answer's content blocks in order, rebuilt verbatim: `text`,
//!   `thinking` with its `signature`, `redacted_thinking` as it came, `tool_use` with its parsed
//!   `input`, and any other block as it started. It is reported just before `Finished`. Empty
//!   text blocks are left out (the API rejects them); empty thinking blocks are kept for their
//!   signatures.
//! - **Usage.** `message_start` and every `message_delta` report counts; each report replaces
//!   the values it names. `input_tokens` is the whole prompt: uncached input plus cache reads
//!   and writes, as OpenAI-compatible servers count it.
//! - **Finish.** `message_stop` reports the replay state and `Finished` with the held stop
//!   reason (`refusal` with its `stop_details`). A body that ends after a stop reason without
//!   `message_stop` still finishes; one that ends before reports nothing more, and the engine
//!   saves the answer as ended early.
//! - **Errors.** An `error` event fails the answer without echoing the provider's text.

use std::collections::BTreeMap;

use copilot_core::{
    DomainError, DomainResult, FinishReason, LlmEvent, PartKind, ProviderId, Refusal, Usage,
};
use serde_json::{Map, Value, json};

use crate::http::{PROVIDER_BAD_RESPONSE, PROVIDER_RATE_LIMITED, bad_response};
use crate::sse::{PROVIDER_ERROR, failed};

/// The most replay state one answer may collect beyond what it streams (signatures,
/// `redacted_thinking` data, other blocks): text, thinking, and arguments count toward the
/// engine's own ceiling.
const REPLAY_MAX_BYTES: usize = 8 * 1_024 * 1_024;

/// The explanation of a refusal whose `stop_details` explain nothing (the same words core uses).
const DECLINED: &str = "The model declined to answer.";

/// What the stream does after an event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Flow {
    Continue,
    /// `message_stop` or a failure: read no further.
    Done,
}

type Out = Vec<DomainResult<LlmEvent>>;

/// The state of one streamed answer.
#[derive(Debug)]
pub(crate) struct Decoder {
    provider: ProviderId,
    /// The next part index.
    next: u32,
    /// The text or reasoning part that deltas of its kind continue.
    run: Option<(PartKind, u32)>,
    /// The content blocks, by the provider's block index.
    blocks: BTreeMap<u64, Block>,
    /// Replay bytes the engine does not count.
    extra_bytes: usize,
    counts: Counts,
    stop: Option<String>,
    stop_details: Option<Value>,
}

/// One content block as it streams.
#[derive(Debug)]
struct Block {
    kind: Kind,
    /// The block as `content_block_start` gave it.
    start: Map<String, Value>,
    /// Text or thinking text.
    text: String,
    signature: String,
    ended: bool,
}

#[derive(Debug)]
enum Kind {
    Text,
    Thinking,
    Tool {
        part: u32,
        /// The `input_json_delta` fragments so far.
        arguments: String,
    },
    /// `redacted_thinking` and any other block: replayed as it started.
    Other,
}

/// The raw token counts reported so far.
#[derive(Debug, Default)]
struct Counts {
    input: Option<u64>,
    cache_read: Option<u64>,
    cache_write: Option<u64>,
    output: Option<u64>,
}

impl Decoder {
    pub(crate) fn new(provider: ProviderId) -> Self {
        Self {
            provider,
            next: 0,
            run: None,
            blocks: BTreeMap::new(),
            extra_bytes: 0,
            counts: Counts::default(),
            stop: None,
            stop_details: None,
        }
    }

    /// Decodes one event: its SSE name and data.
    pub(crate) fn feed(&mut self, name: Option<&str>, data: &str, out: &mut Out) -> Flow {
        match self.event(name, data, out) {
            Ok(flow) => flow,
            Err(error) => {
                out.push(Err(error));
                Flow::Done
            }
        }
    }

    fn event(&mut self, name: Option<&str>, data: &str, out: &mut Out) -> DomainResult<Flow> {
        let data = data.trim();
        if data.is_empty() {
            return match name {
                Some("error") => Err(self.error(&Value::Null)),
                _ => Ok(Flow::Continue),
            };
        }
        let event: Value = serde_json::from_str(data).map_err(|error| {
            tracing::warn!(provider_id = %self.provider, error = %error, "provider.stream.invalid");
            bad_response()
        })?;
        let kind = event.get("type").and_then(Value::as_str).or(name);
        match kind {
            Some("message_start") => {
                if let Some(usage) = event
                    .get("message")
                    .and_then(|message| message.get("usage"))
                {
                    self.usage(usage, out);
                }
            }
            Some("content_block_start") => self.start(&event, out)?,
            Some("content_block_delta") => self.delta(&event, out)?,
            Some("content_block_stop") => {
                if let Some(index) = event.get("index").and_then(Value::as_u64) {
                    self.stop(index, out);
                }
            }
            Some("message_delta") => {
                if let Some(delta) = event.get("delta") {
                    if let Some(reason) = delta.get("stop_reason").and_then(Value::as_str) {
                        self.stop = Some(reason.to_owned());
                    }
                    if let Some(details) =
                        delta.get("stop_details").filter(|value| !value.is_null())
                    {
                        self.stop_details = Some(details.clone());
                    }
                }
                if let Some(usage) = event.get("usage") {
                    self.usage(usage, out);
                }
            }
            Some("message_stop") => {
                self.finished(out);
                return Ok(Flow::Done);
            }
            Some("error") => return Err(self.error(&event)),
            _ => {}
        }
        Ok(Flow::Continue)
    }

    /// `content_block_start`: a new block; a tool call starts its part here.
    fn start(&mut self, event: &Value, out: &mut Out) -> DomainResult<()> {
        let Some(index) = event.get("index").and_then(Value::as_u64) else {
            return Ok(());
        };
        let Some(Value::Object(start)) = event.get("content_block") else {
            return Ok(());
        };
        if self.blocks.contains_key(&index) {
            return Ok(());
        }
        let start = start.clone();
        let kind = match start.get("type").and_then(Value::as_str) {
            Some("text") => Kind::Text,
            Some("thinking") => Kind::Thinking,
            Some("tool_use") => {
                let part = self.allocate();
                self.run = None;
                out.push(Ok(LlmEvent::ToolCallStarted {
                    index: part,
                    provider_call_id: start
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned),
                    alias: start
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                }));
                Kind::Tool {
                    part,
                    arguments: String::new(),
                }
            }
            _ => {
                self.keep(Value::Object(start.clone()).to_string().len())?;
                Kind::Other
            }
        };
        let shown = match kind {
            Kind::Text => Some(PartKind::Text),
            Kind::Thinking => Some(PartKind::Reasoning),
            Kind::Tool { .. } | Kind::Other => None,
        };
        if self.run.map(|(open, _)| open) != shown {
            self.run = None;
        }
        let mut block = Block {
            kind,
            start,
            text: String::new(),
            signature: String::new(),
            ended: false,
        };
        // A start block may already hold text (a gateway that does not stream it).
        let initial = match block.kind {
            Kind::Text => block.start.get("text"),
            Kind::Thinking => block.start.get("thinking"),
            _ => None,
        }
        .and_then(Value::as_str)
        .map(str::to_owned);
        if let Some(signature) = block.start.get("signature").and_then(Value::as_str) {
            self.keep(signature.len())?;
            block.signature.push_str(signature);
        }
        self.blocks.insert(index, block);
        if let Some(text) = initial {
            self.text(index, &text, out);
        }
        Ok(())
    }

    /// `content_block_delta`: text, thinking, a signature, or a piece of tool arguments.
    fn delta(&mut self, event: &Value, out: &mut Out) -> DomainResult<()> {
        let Some(index) = event.get("index").and_then(Value::as_u64) else {
            return Ok(());
        };
        let Some(delta) = event.get("delta") else {
            return Ok(());
        };
        let field = |name: &str| delta.get(name).and_then(Value::as_str);
        match delta.get("type").and_then(Value::as_str) {
            Some("text_delta") => {
                if let Some(text) = field("text") {
                    self.text(index, text, out);
                }
            }
            Some("thinking_delta") => {
                if let Some(text) = field("thinking") {
                    self.text(index, text, out);
                }
            }
            Some("signature_delta") => {
                if let Some(signature) = field("signature") {
                    self.keep(signature.len())?;
                    if let Some(block) = self.blocks.get_mut(&index) {
                        block.signature.push_str(signature);
                    }
                }
            }
            Some("input_json_delta") => {
                if let Some(json) = field("partial_json").filter(|json| !json.is_empty())
                    && let Some(Block {
                        kind: Kind::Tool { part, arguments },
                        ended: false,
                        ..
                    }) = self.blocks.get_mut(&index)
                {
                    arguments.push_str(json);
                    out.push(Ok(LlmEvent::ToolCallArgs {
                        index: *part,
                        json: json.to_owned(),
                    }));
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Text or thinking text for block `index`: kept for replay and shown when not empty.
    fn text(&mut self, index: u64, text: &str, out: &mut Out) {
        let Some(block) = self.blocks.get_mut(&index) else {
            return;
        };
        let kind = match block.kind {
            Kind::Text => PartKind::Text,
            Kind::Thinking => PartKind::Reasoning,
            Kind::Tool { .. } | Kind::Other => return,
        };
        block.text.push_str(text);
        if text.is_empty() {
            return;
        }
        let part = self.part(kind);
        let text = text.to_owned();
        out.push(Ok(match kind {
            PartKind::Reasoning => LlmEvent::ReasoningDelta { index: part, text },
            _ => LlmEvent::TextDelta { index: part, text },
        }));
    }

    /// `content_block_stop`: a tool call ends; its arguments come from the start block when no
    /// fragment came.
    fn stop(&mut self, index: u64, out: &mut Out) {
        let Some(block) = self.blocks.get_mut(&index) else {
            return;
        };
        if block.ended {
            return;
        }
        block.ended = true;
        if let Kind::Tool { part, arguments } = &mut block.kind {
            if arguments.is_empty()
                && let Some(Value::Object(input)) = block.start.get("input")
                && !input.is_empty()
            {
                let json = Value::Object(input.clone()).to_string();
                arguments.push_str(&json);
                out.push(Ok(LlmEvent::ToolCallArgs { index: *part, json }));
            }
            out.push(Ok(LlmEvent::ToolCallEnded { index: *part }));
        }
    }

    /// Reports the counts after `usage` replaces the values it names.
    fn usage(&mut self, usage: &Value, out: &mut Out) {
        let count = |field: &str| usage.get(field).and_then(Value::as_u64);
        let counts = &mut self.counts;
        let mut reported = false;
        for (slot, field) in [
            (&mut counts.input, "input_tokens"),
            (&mut counts.cache_read, "cache_read_input_tokens"),
            (&mut counts.cache_write, "cache_creation_input_tokens"),
            (&mut counts.output, "output_tokens"),
        ] {
            if let Some(value) = count(field) {
                *slot = Some(value);
                reported = true;
            }
        }
        if reported {
            out.push(Ok(LlmEvent::Usage(counts.to_usage())));
        }
    }

    /// The body ended without `message_stop`: the answer is finished only if a stop reason
    /// came.
    pub(crate) fn end(&mut self, out: &mut Out) {
        if self.stop.is_some() {
            self.finished(out);
        }
    }

    /// Ends open tool calls, reports the replay state, then `Finished` with the stop reason.
    fn finished(&mut self, out: &mut Out) {
        let indexes: Vec<u64> = self.blocks.keys().copied().collect();
        for index in indexes {
            self.stop(index, out);
        }
        let content: Vec<Value> = self.blocks.values().filter_map(Block::replay).collect();
        if !content.is_empty() {
            out.push(Ok(LlmEvent::ReplayState(Value::Array(content))));
        }
        let calls = self
            .blocks
            .values()
            .any(|block| matches!(block.kind, Kind::Tool { .. }));
        let (reason, refusal) = match self.stop.as_deref() {
            Some("refusal") => (FinishReason::Refusal, Some(self.refusal())),
            Some(reason) => (finish(reason), None),
            None if calls => (FinishReason::ToolCalls, None),
            None => (FinishReason::Other, None),
        };
        out.push(Ok(LlmEvent::Finished { reason, refusal }));
    }

    /// The refusal `stop_details` describe (`category` and `explanation`, either may be null).
    fn refusal(&self) -> Refusal {
        let details = self.stop_details.as_ref();
        let field = |name: &str| {
            details
                .and_then(|details| details.get(name))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        Refusal {
            category: field("category"),
            explanation: field("explanation").unwrap_or_else(|| DECLINED.to_owned()),
        }
    }

    /// An `error` event: overloaded and rate-limited answers say so; anything else is the
    /// generic failure. The provider's message is never echoed.
    fn error(&self, event: &Value) -> DomainError {
        let kind = event
            .get("error")
            .and_then(|error| error.get("type"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let kind: String = kind.chars().take(64).collect();
        tracing::warn!(provider_id = %self.provider, provider_code = %kind, "provider.stream.error");
        match kind.as_str() {
            "overloaded_error" => DomainError::upstream(
                "The provider is overloaded. Try again in a moment.",
                PROVIDER_ERROR,
            ),
            "rate_limit_error" => DomainError::upstream(
                "The provider is rate limiting requests. Try again later.",
                PROVIDER_RATE_LIMITED,
            ),
            _ => failed(),
        }
    }

    /// The part a delta of `kind` goes to: the open run of that kind, else a new part.
    fn part(&mut self, kind: PartKind) -> u32 {
        match self.run {
            Some((open, index)) if open == kind => index,
            _ => {
                let index = self.allocate();
                self.run = Some((kind, index));
                index
            }
        }
    }

    fn allocate(&mut self) -> u32 {
        let index = self.next;
        self.next = self.next.saturating_add(1);
        index
    }

    /// Counts `bytes` more of replay state the engine does not count, or fails past
    /// [`REPLAY_MAX_BYTES`].
    fn keep(&mut self, bytes: usize) -> DomainResult<()> {
        self.extra_bytes = self.extra_bytes.saturating_add(bytes);
        if self.extra_bytes > REPLAY_MAX_BYTES {
            tracing::warn!(
                provider_id = %self.provider,
                limit = REPLAY_MAX_BYTES,
                "provider.stream.too-large"
            );
            return Err(DomainError::upstream(
                "The provider sent a reply over the size limit.",
                PROVIDER_BAD_RESPONSE,
            ));
        }
        Ok(())
    }
}

impl Block {
    /// The block as it goes back in a replayed answer; none for empty text.
    fn replay(&self) -> Option<Value> {
        match &self.kind {
            Kind::Text => {
                (!self.text.is_empty()).then(|| json!({ "type": "text", "text": self.text }))
            }
            Kind::Thinking => Some(json!({
                "type": "thinking",
                "thinking": self.text,
                "signature": self.signature,
            })),
            Kind::Tool { arguments, .. } => {
                let input = serde_json::from_str::<Value>(arguments)
                    .ok()
                    .filter(Value::is_object)
                    .unwrap_or_else(|| Value::Object(Map::new()));
                Some(json!({
                    "type": "tool_use",
                    "id": self.start.get("id").cloned().unwrap_or(Value::Null),
                    "name": self.start.get("name").cloned().unwrap_or(Value::Null),
                    "input": input,
                }))
            }
            Kind::Other => Some(Value::Object(self.start.clone())),
        }
    }
}

impl Counts {
    fn to_usage(&self) -> Usage {
        let count = |value: Option<u64>| value.and_then(|value| u32::try_from(value).ok());
        let prompt = [self.input, self.cache_read, self.cache_write];
        let input = prompt.iter().any(Option::is_some).then(|| {
            prompt
                .iter()
                .flatten()
                .fold(0_u64, |sum, value| sum.saturating_add(*value))
        });
        Usage {
            input_tokens: count(input),
            output_tokens: count(self.output),
            reasoning_tokens: None,
            cache_read_tokens: count(self.cache_read),
            cache_write_tokens: count(self.cache_write),
        }
    }
}

/// `stop_reason` to the core reason (`refusal` is handled with its details).
fn finish(reason: &str) -> FinishReason {
    match reason {
        "end_turn" | "stop_sequence" => FinishReason::Stop,
        "tool_use" => FinishReason::ToolCalls,
        "max_tokens" | "model_context_window_exceeded" => FinishReason::Length,
        "refusal" => FinishReason::Refusal,
        _ => FinishReason::Other,
    }
}

#[cfg(test)]
mod tests;
