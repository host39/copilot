//! Responses API stream events to `LlmEvent`s (`docs/research/providers.md`, OpenAI Responses).
//!
//! Every event's data names its `type` (OpenAI also sends it as the SSE event name; the decoder
//! reads the data, so servers that send data-only events work too). Events of one answer:
//! `response.created`, then per output item `response.output_item.added`, its deltas, and
//! `response.output_item.done`, then one of `response.completed`, `response.incomplete`, or
//! `response.failed`. `error` ends the answer early. Unknown events are skipped.
//!
//! - **Parts.** Output text and reasoning (summary or raw reasoning text) each continue their
//!   part while deltas of their kind follow one another; anything else ends the run, and a
//!   function call is a part of its own. A new summary part of the same reasoning item starts a
//!   new paragraph. Indexes count up from 0 in arrival order.
//! - **Function calls** start at `response.output_item.added` (`call_id` and `name`), take their
//!   arguments from `response.function_call_arguments.delta` (or from `.done`, or the finished
//!   item, when no delta came), and end at `response.output_item.done`.
//! - **Whole items.** A server that skips deltas still shows its answer: an item finished with
//!   text, a refusal, or a summary that streamed nothing reports it at `output_item.done`, and the
//!   terminal event's `output` fills in items that never came.
//! - **Refusals.** `response.refusal.delta` (or a `refusal` content part) makes the answer a
//!   refusal with that text as the explanation; an `incomplete` answer stopped by
//!   `content_filter` is a refusal without one.
//! - **Replay state** is the finished output items (`replay::state`), reported just before
//!   `Finished`.
//! - **Usage** comes with the terminal event: `input_tokens` (the whole prompt, cached tokens
//!   included), `output_tokens` (reasoning included), and their details.
//! - **Finish.** `response.completed` finishes with `tool_calls` when the answer called a
//!   function, else `stop`; `response.incomplete` with `length` for `max_output_tokens`. A body
//!   that ends without a terminal event reports nothing more, and the engine saves the answer as
//!   ended early.
//! - **Errors.** `response.failed` and `error` fail the answer without echoing the provider's
//!   text; a rate limit keeps its own code.

use std::collections::BTreeMap;

use copilot_core::{
    DomainError, DomainResult, FinishReason, LlmEvent, PartKind, ProviderId, Refusal, Usage,
};
use serde_json::Value;

use crate::http::{PROVIDER_BAD_RESPONSE, PROVIDER_RATE_LIMITED, bad_response};
use crate::responses::replay;
use crate::sse::failed;

/// The most replay state one answer may collect (finished items, encrypted reasoning
/// included): what core stores in `provider_state` is not counted by the engine's ceiling.
const REPLAY_MAX_BYTES: usize = 8 * 1_024 * 1_024;

/// What the stream does after an event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Flow {
    Continue,
    /// A terminal event or a failure: read no further.
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
    /// The output items, by output index.
    items: BTreeMap<u64, Item>,
    /// Bytes of finished items kept for replay.
    replay_bytes: usize,
    /// Refusal text so far.
    refusal: String,
}

/// One output item as it streams.
#[derive(Debug, Default)]
struct Item {
    kind: Kind,
    /// Text or reasoning of this item was shown.
    shown: bool,
    /// Refusal text of this item was taken.
    refused: bool,
    /// The summary part the last reasoning delta belonged to.
    summary: Option<u64>,
    /// The item as `response.output_item.done` finished it.
    finished: Option<Value>,
}

#[derive(Debug, Default)]
enum Kind {
    #[default]
    Other,
    Message,
    Reasoning,
    Call {
        part: u32,
        /// Argument text sent so far.
        arguments: String,
        ended: bool,
    },
}

impl Decoder {
    pub(crate) fn new(provider: ProviderId) -> Self {
        Self {
            provider,
            next: 0,
            run: None,
            items: BTreeMap::new(),
            replay_bytes: 0,
            refusal: String::new(),
        }
    }

    /// Decodes one event: its SSE name (used when the data names no `type`) and data.
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
        if data.is_empty() || data == "[DONE]" {
            return match name {
                Some("error") => Err(self.error(&Value::Null)),
                _ if data == "[DONE]" => Ok(Flow::Done),
                _ => Ok(Flow::Continue),
            };
        }
        let event: Value = serde_json::from_str(data).map_err(|error| {
            tracing::warn!(provider_id = %self.provider, error = %error, "provider.stream.invalid");
            bad_response()
        })?;
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .or(name)
            .unwrap_or_default();
        let output = event.get("output_index").and_then(Value::as_u64);
        let delta = || {
            event
                .get("delta")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
        };
        match kind {
            "response.output_item.added" => {
                if let (Some(output), Some(item)) = (output, event.get("item")) {
                    self.added(output, item, out);
                }
            }
            "response.output_item.done" => {
                if let (Some(output), Some(item)) = (output, event.get("item")) {
                    self.done(output, item.clone(), out)?;
                }
            }
            "response.output_text.delta" => {
                if let (Some(output), Some(text)) = (output, delta()) {
                    self.text(output, text, out);
                }
            }
            "response.output_text.done" => {
                let text = event.get("text").and_then(Value::as_str);
                if let (Some(output), Some(text)) = (output, text)
                    && !self.item(output, Kind::Message).shown
                {
                    self.text(output, text, out);
                }
            }
            "response.refusal.delta" => {
                if let (Some(output), Some(text)) = (output, delta()) {
                    self.refuse(output, text);
                }
            }
            "response.refusal.done" => {
                let text = event.get("refusal").and_then(Value::as_str);
                if let (Some(output), Some(text)) = (output, text)
                    && !self.item(output, Kind::Message).refused
                {
                    self.refuse(output, text);
                }
            }
            "response.reasoning_summary_text.delta" => {
                let summary = event.get("summary_index").and_then(Value::as_u64);
                if let (Some(output), Some(text)) = (output, delta()) {
                    self.reasoning(output, summary, text, out);
                }
            }
            "response.reasoning_summary_text.done" => {
                let summary = event.get("summary_index").and_then(Value::as_u64);
                let text = event.get("text").and_then(Value::as_str);
                if let (Some(output), Some(text)) = (output, text) {
                    let item = self.item(output, Kind::Reasoning);
                    if !item.shown || item.summary != summary {
                        self.reasoning(output, summary, text, out);
                    }
                }
            }
            "response.reasoning_text.delta" => {
                if let (Some(output), Some(text)) = (output, delta()) {
                    self.reasoning(output, None, text, out);
                }
            }
            "response.function_call_arguments.delta" => {
                if let (Some(output), Some(json)) = (output, delta()) {
                    self.arguments(output, json, out);
                }
            }
            "response.function_call_arguments.done" => {
                let json = event.get("arguments").and_then(Value::as_str);
                if let (Some(output), Some(json)) = (output, json)
                    && self.streamed_arguments(output) == Some(false)
                {
                    self.arguments(output, json, out);
                }
            }
            "response.completed" | "response.incomplete" => {
                let response = event.get("response").unwrap_or(&Value::Null);
                self.finished(kind, response, out)?;
                return Ok(Flow::Done);
            }
            "response.failed" => {
                let error = event
                    .get("response")
                    .and_then(|response| response.get("error"))
                    .unwrap_or(&Value::Null);
                return Err(self.error(error));
            }
            "error" => return Err(self.error(&event)),
            _ => {}
        }
        Ok(Flow::Continue)
    }

    /// An item starts: a function call starts its part at once.
    fn added(&mut self, output: u64, item: &Value, out: &mut Out) {
        if self.items.contains_key(&output) {
            return;
        }
        let kind = match item.get("type").and_then(Value::as_str) {
            Some("message") => Kind::Message,
            Some("reasoning") => Kind::Reasoning,
            Some("function_call") => self.start_call(item, out),
            _ => Kind::Other,
        };
        self.items.insert(
            output,
            Item {
                kind,
                ..Item::default()
            },
        );
    }

    /// Starts the part of a function call.
    fn start_call(&mut self, item: &Value, out: &mut Out) -> Kind {
        let part = self.allocate();
        self.run = None;
        let field = |name: &str| item.get(name).and_then(Value::as_str).map(str::to_owned);
        out.push(Ok(LlmEvent::ToolCallStarted {
            index: part,
            provider_call_id: field("call_id"),
            alias: field("name").unwrap_or_default(),
        }));
        Kind::Call {
            part,
            arguments: String::new(),
            ended: false,
        }
    }

    /// An item is finished: keep it for replay, show what streamed nothing, end a call.
    fn done(&mut self, output: u64, item: Value, out: &mut Out) -> DomainResult<()> {
        if self
            .items
            .get(&output)
            .is_some_and(|known| known.finished.is_some())
        {
            return Ok(());
        }
        if !self.items.contains_key(&output) {
            self.added(output, &item, out);
        }
        match item.get("type").and_then(Value::as_str) {
            Some("message") => self.whole_message(output, &item, out),
            Some("reasoning") => self.whole_reasoning(output, &item, out),
            Some("function_call") => {
                if self.streamed_arguments(output) == Some(false)
                    && let Some(json) = item.get("arguments").and_then(Value::as_str)
                {
                    self.arguments(output, json, out);
                }
                self.end_call(output, out);
            }
            _ => {}
        }
        self.keep(item.to_string().len())?;
        if let Some(known) = self.items.get_mut(&output) {
            known.finished = Some(item);
        }
        Ok(())
    }

    /// The text and the refusal of a finished message, each when none of it streamed.
    fn whole_message(&mut self, output: u64, item: &Value, out: &mut Out) {
        let known = self.item(output, Kind::Message);
        let (shown, refused) = (known.shown, known.refused);
        let parts = item.get("content").and_then(Value::as_array);
        for part in parts.into_iter().flatten() {
            match part.get("type").and_then(Value::as_str) {
                Some("output_text") if !shown => {
                    if let Some(text) = part.get("text").and_then(Value::as_str)
                        && !text.is_empty()
                    {
                        self.text(output, text, out);
                    }
                }
                Some("refusal") if !refused => {
                    if let Some(text) = part.get("refusal").and_then(Value::as_str) {
                        self.refuse(output, text);
                    }
                }
                _ => {}
            }
        }
    }

    /// Refusal text of the message at `output`.
    fn refuse(&mut self, output: u64, text: &str) {
        self.item(output, Kind::Message).refused = true;
        self.refusal.push_str(text);
    }

    /// The summary of a finished reasoning item that streamed none.
    fn whole_reasoning(&mut self, output: u64, item: &Value, out: &mut Out) {
        if self.item(output, Kind::Reasoning).shown {
            return;
        }
        let summaries = item.get("summary").and_then(Value::as_array);
        for (index, summary) in summaries.into_iter().flatten().enumerate() {
            if let Some(text) = summary.get("text").and_then(Value::as_str)
                && !text.is_empty()
            {
                self.reasoning(output, Some(index as u64), text, out);
            }
        }
    }

    fn text(&mut self, output: u64, text: &str, out: &mut Out) {
        self.item(output, Kind::Message).shown = true;
        let index = self.part(PartKind::Text);
        out.push(Ok(LlmEvent::TextDelta {
            index,
            text: text.to_owned(),
        }));
    }

    /// A reasoning delta; a new summary part of a shown item starts a paragraph.
    fn reasoning(&mut self, output: u64, summary: Option<u64>, text: &str, out: &mut Out) {
        let item = self.item(output, Kind::Reasoning);
        let paragraph = item.shown && summary.is_some() && item.summary != summary;
        item.shown = true;
        item.summary = summary;
        let index = self.part(PartKind::Reasoning);
        let text = if paragraph {
            format!("\n\n{text}")
        } else {
            text.to_owned()
        };
        out.push(Ok(LlmEvent::ReasoningDelta { index, text }));
    }

    /// Arguments of the call at `output`; nothing when it is not a call or already ended.
    fn arguments(&mut self, output: u64, json: &str, out: &mut Out) {
        if let Some(Item {
            kind:
                Kind::Call {
                    part,
                    arguments,
                    ended: false,
                },
            ..
        }) = self.items.get_mut(&output)
        {
            arguments.push_str(json);
            out.push(Ok(LlmEvent::ToolCallArgs {
                index: *part,
                json: json.to_owned(),
            }));
        }
    }

    /// Whether the call at `output` got arguments already; `None` when it is not a call.
    fn streamed_arguments(&self, output: u64) -> Option<bool> {
        match self.items.get(&output).map(|item| &item.kind) {
            Some(Kind::Call { arguments, .. }) => Some(!arguments.is_empty()),
            _ => None,
        }
    }

    fn end_call(&mut self, output: u64, out: &mut Out) {
        if let Some(Item {
            kind: Kind::Call { part, ended, .. },
            ..
        }) = self.items.get_mut(&output)
            && !*ended
        {
            *ended = true;
            out.push(Ok(LlmEvent::ToolCallEnded { index: *part }));
        }
    }

    /// The item at `output`, created as `kind` when no event named it yet.
    fn item(&mut self, output: u64, kind: Kind) -> &mut Item {
        self.items.entry(output).or_insert_with(|| Item {
            kind,
            ..Item::default()
        })
    }

    /// The terminal event: items it holds that never came, usage, replay state, and `Finished`.
    fn finished(&mut self, kind: &str, response: &Value, out: &mut Out) -> DomainResult<()> {
        let outputs = response.get("output").and_then(Value::as_array);
        for (output, item) in outputs.into_iter().flatten().enumerate() {
            let output = output as u64;
            if self
                .items
                .get(&output)
                .is_none_or(|known| known.finished.is_none())
            {
                self.done(output, item.clone(), out)?;
            }
        }
        let outputs: Vec<u64> = self.items.keys().copied().collect();
        for output in outputs {
            self.end_call(output, out);
        }
        if let Some(usage) = response.get("usage").filter(|usage| usage.is_object()) {
            out.push(Ok(LlmEvent::Usage(read_usage(usage))));
        }
        let finished: Vec<Value> = self
            .items
            .values()
            .filter_map(|item| item.finished.clone())
            .collect();
        if let Some(state) = replay::state(finished) {
            out.push(Ok(LlmEvent::ReplayState(state)));
        }
        let calls = self
            .items
            .values()
            .any(|item| matches!(item.kind, Kind::Call { .. }));
        let incomplete = response
            .get("incomplete_details")
            .and_then(|details| details.get("reason"))
            .and_then(Value::as_str);
        let refusal = self.refusal.trim();
        let (reason, refusal) = if !refusal.is_empty() {
            let refusal = Refusal {
                category: None,
                explanation: refusal.to_owned(),
            };
            (FinishReason::Refusal, Some(refusal))
        } else if kind == "response.incomplete" {
            let reason = match incomplete {
                Some("max_output_tokens") => FinishReason::Length,
                Some("content_filter") => FinishReason::Refusal,
                _ => FinishReason::Other,
            };
            (reason, None)
        } else if calls {
            (FinishReason::ToolCalls, None)
        } else {
            (FinishReason::Stop, None)
        };
        out.push(Ok(LlmEvent::Finished { reason, refusal }));
        Ok(())
    }

    /// The error of a failed answer (`response.error`) or an `error` event; its message is not
    /// echoed.
    fn error(&self, error: &Value) -> DomainError {
        let code = error
            .get("code")
            .and_then(Value::as_str)
            .or_else(|| error.get("type").and_then(Value::as_str))
            .unwrap_or_default();
        let code: String = code.chars().take(64).collect();
        tracing::warn!(provider_id = %self.provider, provider_code = %code, "provider.stream.error");
        match code.as_str() {
            "rate_limit_exceeded" => DomainError::upstream(
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

    /// Counts `bytes` more of replay state, or fails past [`REPLAY_MAX_BYTES`].
    fn keep(&mut self, bytes: usize) -> DomainResult<()> {
        self.replay_bytes = self.replay_bytes.saturating_add(bytes);
        if self.replay_bytes > REPLAY_MAX_BYTES {
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

/// The token counts of a terminal event's `usage`.
fn read_usage(usage: &Value) -> Usage {
    let count = |value: Option<&Value>| {
        value
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
    };
    let input = usage.get("input_tokens_details");
    let output = usage.get("output_tokens_details");
    Usage {
        input_tokens: count(usage.get("input_tokens")),
        output_tokens: count(usage.get("output_tokens")),
        reasoning_tokens: count(output.and_then(|details| details.get("reasoning_tokens"))),
        cache_read_tokens: count(input.and_then(|details| details.get("cached_tokens"))),
        cache_write_tokens: count(input.and_then(|details| details.get("cache_write_tokens"))),
    }
}

#[cfg(test)]
mod tests;
