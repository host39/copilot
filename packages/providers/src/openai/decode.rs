//! Chat Completions stream chunks to `LlmEvent`s (`docs/research/providers.md`,
//! OpenAI-Compatible, Streaming).
//!
//! Each SSE event is `data: <chat.completion.chunk JSON>`; `data: [DONE]` ends the stream. The
//! finish reason arrives before a final usage chunk (`stream_options.include_usage`), so the
//! decoder holds it and reports `Finished` at `[DONE]`, or when the body ends after a finish
//! reason without one. A body that ends with no finish reports nothing more, and the engine
//! saves the answer as ended early. OpenRouter's mid-stream error chunk (a top-level `error`)
//! fails the answer.

use copilot_core::{DomainError, DomainResult, FinishReason, LlmEvent, ProviderId, Refusal, Usage};
use serde::Deserialize;
use serde_json::Value;

use crate::http::bad_response;

/// The code of an answer the provider ended with an error chunk.
pub(crate) const PROVIDER_ERROR: &str = "PROVIDER_ERROR";

/// The part index of the answer's text.
const TEXT: u32 = 0;

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
}

#[derive(Debug, Deserialize)]
struct Choice {
    #[serde(default)]
    index: u32,
    #[serde(default)]
    delta: Option<Delta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    refusal: Option<String>,
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

impl Decoder {
    pub(crate) fn new(provider: ProviderId) -> Self {
        Self {
            provider,
            finish: None,
            refusal: String::new(),
            filtered: false,
        }
    }

    /// Decodes one event's data into `out`.
    pub(crate) fn feed(&mut self, data: &str, out: &mut Vec<DomainResult<LlmEvent>>) -> Flow {
        let data = data.trim();
        if data.is_empty() {
            return Flow::Continue;
        }
        if data == "[DONE]" {
            out.push(Ok(self.finished(FinishReason::Other)));
            return Flow::Done;
        }
        let chunk: Chunk = match serde_json::from_str(data) {
            Ok(chunk) => chunk,
            Err(error) => {
                tracing::warn!(provider_id = %self.provider, error = %error, "provider.stream.invalid");
                out.push(Err(bad_response()));
                return Flow::Done;
            }
        };
        if let Some(error) = chunk.error {
            let code = error.get("code").map(ToString::to_string);
            tracing::warn!(
                provider_id = %self.provider,
                provider_code = code.as_deref(),
                "provider.stream.error"
            );
            out.push(Err(self.failed()));
            return Flow::Done;
        }
        for choice in chunk.choices.into_iter().filter(|choice| choice.index == 0) {
            if let Some(delta) = choice.delta {
                if let Some(text) = delta.content.filter(|text| !text.is_empty()) {
                    out.push(Ok(LlmEvent::TextDelta { index: TEXT, text }));
                }
                if let Some(refusal) = delta.refusal {
                    self.refusal.push_str(&refusal);
                }
            }
            if let Some(reason) = choice.finish_reason {
                self.filtered |= reason == "content_filter";
                self.finish = Some(finish(&reason));
            }
        }
        if let Some(usage) = chunk.usage {
            out.push(Ok(LlmEvent::Usage(usage.into())));
        }
        Flow::Continue
    }

    /// The provider ended the answer with an error (an error chunk or an `error` event). Its
    /// text is not echoed.
    pub(crate) fn failed(&self) -> DomainError {
        DomainError::upstream(
            "The provider ended the answer with an error.",
            PROVIDER_ERROR,
        )
    }

    /// The body ended without `[DONE]`: the answer is finished only if a finish reason came.
    pub(crate) fn end(&mut self) -> Option<LlmEvent> {
        self.finish
            .is_some()
            .then(|| self.finished(FinishReason::Other))
    }

    /// `Finished` with the held reason (`fallback` when none came); a refusal text or a content
    /// filter makes it a refusal.
    fn finished(&mut self, fallback: FinishReason) -> LlmEvent {
        let refusal = self.refusal.trim();
        if !refusal.is_empty() || self.filtered {
            let refusal = (!refusal.is_empty()).then(|| Refusal {
                category: None,
                explanation: refusal.to_owned(),
            });
            return LlmEvent::Finished {
                reason: FinishReason::Refusal,
                refusal,
            };
        }
        LlmEvent::Finished {
            reason: self.finish.unwrap_or(fallback),
            refusal: None,
        }
    }
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
mod tests {
    use super::*;

    fn decode(events: &[&str]) -> (Vec<DomainResult<LlmEvent>>, Flow) {
        let mut decoder = Decoder::new(ProviderId::generate());
        let mut out = Vec::new();
        let mut flow = Flow::Continue;
        for data in events {
            flow = decoder.feed(data, &mut out);
            if flow == Flow::Done {
                break;
            }
        }
        if flow == Flow::Continue
            && let Some(event) = decoder.end()
        {
            out.push(Ok(event));
        }
        (out, flow)
    }

    fn ok(events: Vec<DomainResult<LlmEvent>>) -> Vec<LlmEvent> {
        events
            .into_iter()
            .map(|event| event.expect("no failure"))
            .collect()
    }

    fn text(text: &str) -> LlmEvent {
        LlmEvent::TextDelta {
            index: TEXT,
            text: text.to_owned(),
        }
    }

    fn finished(reason: FinishReason) -> LlmEvent {
        LlmEvent::Finished {
            reason,
            refusal: None,
        }
    }

    #[test]
    fn streams_text_then_finishes_at_done() {
        let (events, flow) = decode(&[
            r#"{"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            r#"{"choices":[{"index":0,"delta":{"content":"Hel"},"finish_reason":null}]}"#,
            r#"{"choices":[{"index":0,"delta":{"content":"lo"},"finish_reason":null}]}"#,
            r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ]);
        assert_eq!(flow, Flow::Done);
        assert_eq!(
            ok(events),
            [text("Hel"), text("lo"), finished(FinishReason::Stop)]
        );
    }

    #[test]
    fn keeps_the_finish_for_the_usage_chunk_after_it() {
        let (events, _) = decode(&[
            r#"{"choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":"length"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":82,"completion_tokens":18,"total_tokens":100,"completion_tokens_details":{"reasoning_tokens":4},"prompt_tokens_details":{"cached_tokens":64,"cache_write_tokens":8}}}"#,
            "[DONE]",
        ]);
        assert_eq!(
            ok(events),
            [
                text("Hi"),
                LlmEvent::Usage(Usage {
                    input_tokens: Some(82),
                    output_tokens: Some(18),
                    reasoning_tokens: Some(4),
                    cache_read_tokens: Some(64),
                    cache_write_tokens: Some(8),
                }),
                finished(FinishReason::Length),
            ]
        );
    }

    #[test]
    fn reads_flat_cached_tokens_and_ignores_counts_out_of_range() {
        let (events, _) = decode(&[
            r#"{"choices":[],"usage":{"prompt_tokens":99999999999,"completion_tokens":3,"cached_tokens":2}}"#,
        ]);
        assert_eq!(
            ok(events),
            [LlmEvent::Usage(Usage {
                input_tokens: None,
                output_tokens: Some(3),
                cache_read_tokens: Some(2),
                ..Usage::default()
            })]
        );
    }

    #[test]
    fn maps_every_finish_reason() {
        for (wire, reason) in [
            ("stop", FinishReason::Stop),
            ("length", FinishReason::Length),
            ("tool_calls", FinishReason::ToolCalls),
            ("function_call", FinishReason::ToolCalls),
            ("error", FinishReason::Error),
            ("something_new", FinishReason::Other),
        ] {
            let chunk =
                format!(r#"{{"choices":[{{"index":0,"delta":{{}},"finish_reason":"{wire}"}}]}}"#);
            let (events, _) = decode(&[&chunk, "[DONE]"]);
            assert_eq!(ok(events), [finished(reason)], "{wire}");
        }
    }

    #[test]
    fn a_content_filter_or_refusal_text_is_a_refusal() {
        let (events, _) = decode(&[
            r#"{"choices":[{"index":0,"delta":{"content":"Partial"},"finish_reason":"content_filter"}]}"#,
            "[DONE]",
        ]);
        assert_eq!(
            ok(events),
            [
                text("Partial"),
                LlmEvent::Finished {
                    reason: FinishReason::Refusal,
                    refusal: None,
                }
            ]
        );

        let (events, _) = decode(&[
            r#"{"choices":[{"index":0,"delta":{"refusal":"I can't "},"finish_reason":null}]}"#,
            r#"{"choices":[{"index":0,"delta":{"refusal":"help with that."},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ]);
        assert_eq!(
            ok(events),
            [LlmEvent::Finished {
                reason: FinishReason::Refusal,
                refusal: Some(Refusal {
                    category: None,
                    explanation: "I can't help with that.".to_owned(),
                }),
            }]
        );
    }

    #[test]
    fn a_body_that_ends_after_a_finish_still_finishes() {
        let (events, flow) = decode(&[
            r#"{"choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":"stop"}]}"#,
        ]);
        assert_eq!(flow, Flow::Continue);
        assert_eq!(ok(events), [text("Hi"), finished(FinishReason::Stop)]);
    }

    #[test]
    fn a_body_that_ends_without_a_finish_reports_no_finish() {
        let (events, _) = decode(&[r#"{"choices":[{"index":0,"delta":{"content":"Hi"}}]}"#]);
        assert_eq!(ok(events), [text("Hi")]);
    }

    #[test]
    fn done_without_a_finish_reason_finishes_as_other() {
        let (events, _) = decode(&["[DONE]"]);
        assert_eq!(ok(events), [finished(FinishReason::Other)]);
    }

    #[test]
    fn an_error_chunk_fails_the_answer() {
        let (events, flow) = decode(&[
            r#"{"choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
            r#"{"error":{"code":502,"message":"Provider disconnected unexpectedly"},"choices":[{"index":0,"delta":{"content":""},"finish_reason":"error"}]}"#,
            r#"{"choices":[{"index":0,"delta":{"content":"never read"}}]}"#,
        ]);
        assert_eq!(flow, Flow::Done);
        assert_eq!(events.len(), 2);
        let error = events[1].as_ref().expect_err("the error chunk");
        assert_eq!(error.code(), PROVIDER_ERROR);
        assert!(
            !error.to_string().contains("disconnected"),
            "the provider's text is not echoed"
        );
    }

    #[test]
    fn unreadable_json_fails_the_answer() {
        let (events, flow) = decode(&["{not json"]);
        assert_eq!(flow, Flow::Done);
        assert_eq!(
            events[0].as_ref().expect_err("invalid").code(),
            "PROVIDER_BAD_RESPONSE"
        );
    }

    #[test]
    fn reads_only_the_first_choice_and_skips_empty_data() {
        let (events, _) = decode(&[
            "",
            r#"{"choices":[{"index":1,"delta":{"content":"other"}},{"index":0,"delta":{"content":"mine"}}]}"#,
            r#"{"id":"x","object":"chat.completion.chunk","obfuscation":"abc","choices":[{"index":0,"delta":{"content":null},"logprobs":null,"finish_reason":"stop"}]}"#,
            "[DONE]",
        ]);
        assert_eq!(ok(events), [text("mine"), finished(FinishReason::Stop)]);
    }
}
