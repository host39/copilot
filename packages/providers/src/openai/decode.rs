//! Chat Completions stream chunks to `LlmEvent`s (`docs/research/providers.md`,
//! OpenAI-Compatible, Streaming and Reasoning Output).
//!
//! Each SSE event is `data: <chat.completion.chunk JSON>`; `data: [DONE]` ends the stream.
//!
//! - **Parts.** Text and reasoning each continue their part until another kind of part comes in
//!   between; a tool call is a part of its own. Indexes count up from 0 in arrival order.
//! - **Reasoning** comes under `reasoning` (Ollama, vLLM, Groq, OpenRouter) or
//!   `reasoning_content` (llama.cpp, LM Studio, some Together models), and on OpenRouter also as
//!   `reasoning_details`, whose `reasoning.text` and `reasoning.summary` text is shown when a
//!   chunk has no plain reasoning text. The text, its key, and the details are kept for replay.
//! - **Tool calls** are reassembled by `index` (see `calls`); every call ends at the finish
//!   reason. A `stop` finish with calls reports `tool_calls`. On the Gemini dialect extra call
//!   fields (thought signatures) are kept for replay.
//! - **Spoken replies** (`delta.audio`) report their PCM bytes and transcript pieces.
//! - **Finish.** The finish reason arrives before a final usage chunk
//!   (`stream_options.include_usage`), so the decoder holds it and reports the replay state and
//!   `Finished` at `[DONE]`, or when the body ends after a finish reason without one. A body
//!   that ends with no finish reports nothing more, and the engine saves the answer as ended
//!   early.
//! - **Errors.** A chunk with a top-level `error` (OpenRouter, llama.cpp) or `object: "error"`
//!   (vLLM) fails the answer without echoing the provider's text.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use copilot_core::{
    Dialect, DomainError, DomainResult, FinishReason, LlmEvent, PartKind, ProviderId, Refusal,
    Usage,
};
use serde::Deserialize;
use serde_json::Value;

use crate::http::{PROVIDER_BAD_RESPONSE, bad_response};
use crate::openai::calls::{Calls, Fragment};
use crate::openai::replay::{REASONING, REASONING_CONTENT, ReplayState};

/// The code of an answer the provider ended with an error chunk.
pub(crate) const PROVIDER_ERROR: &str = "PROVIDER_ERROR";

/// The most replay state (reasoning text, `reasoning_details`, extra call fields) one answer
/// may collect: what core stores in `provider_state` is not counted by the engine's ceiling.
const REPLAY_MAX_BYTES: usize = 8 * 1_024 * 1_024;

/// What the stream does after an event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Flow {
    Continue,
    /// `[DONE]` or a failure: read no further.
    Done,
}

/// The state of one streamed answer.
#[derive(Debug)]
pub(crate) struct Decoder {
    provider: ProviderId,
    dialect: Dialect,
    /// The next part index.
    next: u32,
    /// The text or reasoning part that deltas of its kind continue.
    run: Option<(PartKind, u32)>,
    calls: Calls,
    replay: ReplayState,
    /// What `replay` holds, in bytes.
    replay_bytes: usize,
    finish: Option<FinishReason>,
    /// `delta.refusal` text (OpenAI's structured refusal).
    refusal: String,
    /// `content_filter` stopped the answer.
    filtered: bool,
}

#[derive(Debug, Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<WireUsage>,
    #[serde(default)]
    error: Option<Value>,
    #[serde(default)]
    object: Option<Value>,
    #[serde(default)]
    code: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    delta: Option<Delta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

/// The fields a chunk's delta may carry. They are read leniently: a field of an unexpected type
/// is skipped rather than failing the answer.
#[derive(Debug, Default, Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<Value>,
    #[serde(default)]
    refusal: Option<Value>,
    #[serde(default)]
    reasoning: Option<Value>,
    #[serde(default)]
    reasoning_content: Option<Value>,
    #[serde(default)]
    reasoning_details: Option<Value>,
    #[serde(default)]
    tool_calls: Option<Value>,
    #[serde(default)]
    audio: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens_details: Option<CompletionDetails>,
    #[serde(default)]
    prompt_tokens_details: Option<PromptDetails>,
    /// Together reports cached tokens at the top level for some models.
    #[serde(default)]
    cached_tokens: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct CompletionDetails {
    #[serde(default)]
    reasoning_tokens: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct PromptDetails {
    #[serde(default)]
    cached_tokens: Option<u64>,
    #[serde(default)]
    cache_write_tokens: Option<u64>,
}

type Out = Vec<DomainResult<LlmEvent>>;

impl Decoder {
    pub(crate) fn new(provider: ProviderId, dialect: Dialect) -> Self {
        Self {
            provider,
            dialect,
            next: 0,
            run: None,
            calls: Calls::default(),
            replay: ReplayState::default(),
            replay_bytes: 0,
            finish: None,
            refusal: String::new(),
            filtered: false,
        }
    }

    /// Decodes one event's data into `out`.
    pub(crate) fn feed(&mut self, data: &str, out: &mut Out) -> Flow {
        match self.chunk(data, out) {
            Ok(flow) => flow,
            Err(error) => {
                out.push(Err(error));
                Flow::Done
            }
        }
    }

    fn chunk(&mut self, data: &str, out: &mut Out) -> DomainResult<Flow> {
        let data = data.trim();
        if data.is_empty() {
            return Ok(Flow::Continue);
        }
        if data == "[DONE]" {
            self.finished(out);
            return Ok(Flow::Done);
        }
        let chunk: Chunk = serde_json::from_str(data).map_err(|error| {
            tracing::warn!(provider_id = %self.provider, error = %error, "provider.stream.invalid");
            bad_response()
        })?;
        let error_object = chunk.object.as_ref().and_then(Value::as_str) == Some("error");
        if chunk.error.is_some() || error_object {
            let code = chunk
                .error
                .as_ref()
                .and_then(|error| error.get("code"))
                .or(chunk.code.as_ref())
                .map(ToString::to_string);
            tracing::warn!(
                provider_id = %self.provider,
                provider_code = code.as_deref(),
                "provider.stream.error"
            );
            return Err(failed());
        }
        for choice in chunk.choices {
            if choice.index.unwrap_or(0) != 0 {
                continue;
            }
            if let Some(delta) = choice.delta {
                self.delta(delta, out)?;
            }
            if let Some(reason) = choice.finish_reason {
                self.filtered |= reason == "content_filter";
                self.finish = Some(finish(&reason));
                self.end_calls(out);
            }
        }
        if let Some(usage) = chunk.usage {
            out.push(Ok(LlmEvent::Usage(usage.into())));
        }
        Ok(Flow::Continue)
    }

    /// One choice's delta, in the order a reader would see it: reasoning, text, tool calls,
    /// audio.
    fn delta(&mut self, delta: Delta, out: &mut Out) -> DomainResult<()> {
        let plain = [
            (REASONING, delta.reasoning),
            (REASONING_CONTENT, delta.reasoning_content),
        ]
        .into_iter()
        .find_map(|(key, value)| text(value).map(|text| (key, text)));
        let shown = plain.is_some();
        if let Some((key, text)) = plain {
            self.keep(text.len())?;
            self.replay.key.get_or_insert(key);
            self.replay.reasoning.push_str(&text);
            let index = self.part(PartKind::Reasoning);
            out.push(Ok(LlmEvent::ReasoningDelta { index, text }));
        }
        if let Some(Value::Array(fragments)) = delta.reasoning_details {
            for fragment in fragments {
                self.keep(fragment.to_string().len())?;
                let display = if shown { None } else { detail_text(&fragment) };
                self.replay.add_detail(fragment);
                if let Some(text) = display {
                    let index = self.part(PartKind::Reasoning);
                    out.push(Ok(LlmEvent::ReasoningDelta { index, text }));
                }
            }
        }
        if let Some(content) = text(delta.content) {
            let index = self.part(PartKind::Text);
            out.push(Ok(LlmEvent::TextDelta {
                index,
                text: content,
            }));
        }
        if let Some(refusal) = text(delta.refusal) {
            self.refusal.push_str(&refusal);
        }
        if let Some(Value::Array(items)) = delta.tool_calls {
            for fragment in items.into_iter().filter_map(Fragment::read) {
                self.call(fragment, out)?;
            }
        }
        if let Some(audio) = delta.audio {
            self.audio(&audio, out)?;
        }
        Ok(())
    }

    /// One tool call fragment: a new call starts its part; arguments continue it.
    fn call(&mut self, fragment: Fragment, out: &mut Out) -> DomainResult<()> {
        let position = match self.calls.find(&fragment) {
            Some(position) => position,
            None => {
                let part = self.allocate();
                self.run = None;
                out.push(Ok(LlmEvent::ToolCallStarted {
                    index: part,
                    provider_call_id: fragment.id.clone(),
                    alias: fragment.name.clone().unwrap_or_default(),
                }));
                self.calls.start(part, &fragment)
            }
        };
        if self.dialect.keeps_tool_call_extra() && !fragment.extra.is_empty() {
            self.keep(Value::Object(fragment.extra.clone()).to_string().len())?;
            self.calls.get_mut(position).extra.extend(fragment.extra);
        }
        let call = self.calls.get_mut(position);
        if !call.ended && !fragment.arguments.is_empty() {
            out.push(Ok(LlmEvent::ToolCallArgs {
                index: call.part,
                json: fragment.arguments,
            }));
        }
        Ok(())
    }

    /// A spoken reply piece: base64 PCM `data` and a `transcript` piece, either may be missing.
    fn audio(&mut self, audio: &Value, out: &mut Out) -> DomainResult<()> {
        let data = match audio.get("data").and_then(Value::as_str) {
            Some(encoded) if !encoded.is_empty() => STANDARD.decode(encoded).map_err(|error| {
                tracing::warn!(provider_id = %self.provider, error = %error, "provider.stream.invalid");
                bad_response()
            })?,
            _ => Vec::new(),
        };
        let transcript = audio
            .get("transcript")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned);
        if !data.is_empty() || transcript.is_some() {
            out.push(Ok(LlmEvent::AudioDelta {
                data: Bytes::from(data),
                transcript,
            }));
        }
        Ok(())
    }

    /// A named `error` SSE event ended the answer.
    pub(crate) fn error_event(&self) -> DomainError {
        tracing::warn!(provider_id = %self.provider, "provider.stream.error");
        failed()
    }

    /// The body ended without `[DONE]`: the answer is finished only if a finish reason came.
    pub(crate) fn end(&mut self, out: &mut Out) {
        if self.finish.is_some() {
            self.finished(out);
        }
    }

    /// Ends open calls, reports the replay state, then `Finished` with the held reason. A
    /// refusal text or a content filter makes it a refusal; tool calls make a `stop` (or no
    /// reason at all) `tool_calls`.
    fn finished(&mut self, out: &mut Out) {
        self.end_calls(out);
        self.replay.extra = self.calls.extra();
        if !self.replay.is_empty() {
            out.push(Ok(LlmEvent::ReplayState(self.replay.to_value())));
        }
        let refusal = self.refusal.trim();
        if !refusal.is_empty() || self.filtered {
            let refusal = (!refusal.is_empty()).then(|| Refusal {
                category: None,
                explanation: refusal.to_owned(),
            });
            out.push(Ok(LlmEvent::Finished {
                reason: FinishReason::Refusal,
                refusal,
            }));
            return;
        }
        let reason = match self.finish {
            Some(FinishReason::Stop) | None if !self.calls.is_empty() => FinishReason::ToolCalls,
            Some(reason) => reason,
            None => FinishReason::Other,
        };
        out.push(Ok(LlmEvent::Finished {
            reason,
            refusal: None,
        }));
    }

    fn end_calls(&mut self, out: &mut Out) {
        for index in self.calls.end() {
            out.push(Ok(LlmEvent::ToolCallEnded { index }));
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

/// The provider ended the answer with an error (an error chunk, an `error` event, or a JSON
/// error reply to a streamed request). Its text is not echoed.
pub(crate) fn failed() -> DomainError {
    DomainError::upstream(
        "The provider ended the answer with an error.",
        PROVIDER_ERROR,
    )
}

/// A non-empty string field.
fn text(value: Option<Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) if !text.is_empty() => Some(text),
        _ => None,
    }
}

/// The readable text of a `reasoning_details` fragment: `reasoning.text` text or
/// `reasoning.summary` summary. Encrypted fragments show nothing.
fn detail_text(fragment: &Value) -> Option<String> {
    let field = match fragment.get("type").and_then(Value::as_str)? {
        "reasoning.text" => "text",
        "reasoning.summary" => "summary",
        _ => return None,
    };
    fragment
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// `finish_reason` to the core reason.
fn finish(reason: &str) -> FinishReason {
    match reason {
        "stop" => FinishReason::Stop,
        "length" => FinishReason::Length,
        "tool_calls" | "function_call" => FinishReason::ToolCalls,
        "content_filter" => FinishReason::Refusal,
        "error" => FinishReason::Error,
        _ => FinishReason::Other,
    }
}

impl From<WireUsage> for Usage {
    fn from(usage: WireUsage) -> Self {
        let count = |value: Option<u64>| value.and_then(|value| u32::try_from(value).ok());
        let prompt = usage.prompt_tokens_details.unwrap_or_default();
        Self {
            input_tokens: count(usage.prompt_tokens),
            output_tokens: count(usage.completion_tokens),
            reasoning_tokens: count(
                usage
                    .completion_tokens_details
                    .and_then(|details| details.reasoning_tokens),
            ),
            cache_read_tokens: count(prompt.cached_tokens.or(usage.cached_tokens)),
            cache_write_tokens: count(prompt.cache_write_tokens),
        }
    }
}

#[cfg(test)]
mod tests;
