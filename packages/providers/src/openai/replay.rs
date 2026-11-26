//! The provider-opaque state of an OpenAI-compatible answer (`provider_state.content`, see
//! "Stored JSON Shapes" in `docs/design/copilot.md`): the reasoning text and the key it came
//! under, OpenRouter's `reasoning_details`, and extra per-tool-call fields (Gemini thought
//! signatures), keyed by the provider's call id.
//!
//! The decoder writes it once per answer, just before `Finished`; the request builder reads it
//! back when core replays the answer to the same connection. Reading is lenient: a field of the
//! wrong type is ignored, so a stored state never stops a chat, it only replays less.

use serde_json::{Map, Value};

/// The reasoning key Ollama, vLLM, Groq, and OpenRouter use.
pub(crate) const REASONING: &str = "reasoning";
/// The reasoning key llama.cpp, LM Studio, and some Together models use.
pub(crate) const REASONING_CONTENT: &str = "reasoning_content";

const KEY: &str = "reasoning_key";
const DETAILS: &str = "reasoning_details";
const EXTRA: &str = "tool_call_extra";

/// What one answer leaves for its replay.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ReplayState {
    /// [`REASONING`] or [`REASONING_CONTENT`]: the key the text came under and goes back under.
    pub(crate) key: Option<&'static str>,
    pub(crate) reasoning: String,
    /// OpenRouter `reasoning_details`, in order, fragments of one index merged.
    pub(crate) details: Vec<Value>,
    /// Extra fields of each tool call, by the provider's call id.
    pub(crate) extra: Map<String, Value>,
}

impl ReplayState {
    pub(crate) fn is_empty(&self) -> bool {
        self.reasoning.is_empty() && self.details.is_empty() && self.extra.is_empty()
    }

    /// The stored shape: only the fields that hold something.
    pub(crate) fn to_value(&self) -> Value {
        let mut content = Map::new();
        if !self.reasoning.is_empty() {
            content.insert(KEY.to_owned(), Value::from(self.key.unwrap_or(REASONING)));
            content.insert(REASONING.to_owned(), Value::from(self.reasoning.as_str()));
        }
        if !self.details.is_empty() {
            content.insert(DETAILS.to_owned(), Value::Array(self.details.clone()));
        }
        if !self.extra.is_empty() {
            content.insert(EXTRA.to_owned(), Value::Object(self.extra.clone()));
        }
        Value::Object(content)
    }

    /// Adds one streamed `reasoning_details` fragment. A fragment with the same `index` and
    /// `type` as the last detail (or, without an `index`, a `reasoning.text` or
    /// `reasoning.summary` fragment after one of its type) continues it: its `text`, `summary`,
    /// and `data` strings are appended, and its other fields fill what the detail lacks. Any
    /// other fragment is a new detail. Non-objects are ignored.
    pub(crate) fn add_detail(&mut self, fragment: Value) {
        let Value::Object(fragment) = fragment else {
            return;
        };
        if let Some(Value::Object(last)) = self.details.last_mut()
            && continues(last, &fragment)
        {
            for (key, value) in fragment {
                match (last.get_mut(&key), value) {
                    (Some(Value::String(text)), Value::String(more))
                        if matches!(key.as_str(), "text" | "summary" | "data") =>
                    {
                        text.push_str(&more);
                    }
                    (Some(slot @ Value::Null), value) => *slot = value,
                    (None, value) => {
                        last.insert(key, value);
                    }
                    (Some(_), _) => {}
                }
            }
            return;
        }
        self.details.push(Value::Object(fragment));
    }

    /// Reads a stored state; anything unreadable reads as absent.
    pub(crate) fn read(value: &Value) -> Self {
        let Some(content) = value.as_object() else {
            return Self::default();
        };
        let key = match content.get(KEY).and_then(Value::as_str) {
            Some(REASONING_CONTENT) => REASONING_CONTENT,
            _ => REASONING,
        };
        let reasoning = content
            .get(REASONING)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let details = content
            .get(DETAILS)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let extra = content
            .get(EXTRA)
            .and_then(Value::as_object)
            .map(|extra| {
                extra
                    .iter()
                    .filter(|(_, fields)| fields.is_object())
                    .map(|(id, fields)| (id.clone(), fields.clone()))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            key: Some(key),
            reasoning,
            details,
            extra,
        }
    }
}

/// Whether `fragment` continues the detail `last` (see [`ReplayState::add_detail`]).
fn continues(last: &Map<String, Value>, fragment: &Map<String, Value>) -> bool {
    let kind = fragment.get("type");
    if kind.is_some() && last.get("type") != kind {
        return false;
    }
    match fragment.get("index").and_then(Value::as_u64) {
        Some(index) => last.get("index").and_then(Value::as_u64) == Some(index),
        None => matches!(
            last.get("type").and_then(Value::as_str),
            Some("reasoning.text" | "reasoning.summary")
        ),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn merges_fragments_of_one_detail_and_keeps_others_apart() {
        let mut state = ReplayState::default();
        for fragment in [
            json!({ "type": "reasoning.text", "text": "Let me ", "index": 0, "format": "anthropic-claude-v1", "signature": null }),
            json!({ "type": "reasoning.text", "text": "think.", "index": 0 }),
            json!({ "type": "reasoning.text", "text": "", "index": 0, "signature": "sig" }),
            json!({ "type": "reasoning.encrypted", "data": "abc", "index": 1 }),
            json!({ "type": "reasoning.encrypted", "data": "def", "index": 1 }),
            json!({ "type": "reasoning.summary", "summary": "A " }),
            json!({ "type": "reasoning.summary", "summary": "plan." }),
            json!({ "type": "reasoning.encrypted", "data": "x" }),
            json!({ "type": "reasoning.encrypted", "data": "y" }),
            json!("not an object"),
        ] {
            state.add_detail(fragment);
        }
        assert_eq!(
            state.details,
            [
                json!({ "type": "reasoning.text", "text": "Let me think.", "index": 0, "format": "anthropic-claude-v1", "signature": "sig" }),
                json!({ "type": "reasoning.encrypted", "data": "abcdef", "index": 1 }),
                json!({ "type": "reasoning.summary", "summary": "A plan." }),
                json!({ "type": "reasoning.encrypted", "data": "x" }),
                json!({ "type": "reasoning.encrypted", "data": "y" }),
            ]
        );
    }

    #[test]
    fn writes_only_what_it_holds_and_reads_it_back() {
        assert_eq!(ReplayState::default().to_value(), json!({}));
        assert!(ReplayState::default().is_empty());

        let mut extra = Map::new();
        extra.insert(
            "call_1".to_owned(),
            json!({ "extra_content": { "google": { "thought_signature": "sig" } } }),
        );
        let state = ReplayState {
            key: Some(REASONING_CONTENT),
            reasoning: "Thinking.".to_owned(),
            details: vec![json!({ "type": "reasoning.text", "text": "Thinking.", "index": 0 })],
            extra,
        };
        let stored = state.to_value();
        assert_eq!(
            stored,
            json!({
                "reasoning_key": "reasoning_content",
                "reasoning": "Thinking.",
                "reasoning_details": [{ "type": "reasoning.text", "text": "Thinking.", "index": 0 }],
                "tool_call_extra": {
                    "call_1": { "extra_content": { "google": { "thought_signature": "sig" } } },
                },
            })
        );
        assert_eq!(ReplayState::read(&stored), state);
    }

    #[test]
    fn reads_unreadable_fields_as_absent() {
        let state = ReplayState::read(&json!({
            "reasoning_key": "something_else",
            "reasoning": 42,
            "reasoning_details": "not a list",
            "tool_call_extra": { "call_1": "not an object", "call_2": { "a": 1 } },
        }));
        assert_eq!(state.key, Some(REASONING));
        assert!(state.reasoning.is_empty());
        assert!(state.details.is_empty());
        assert_eq!(state.extra.len(), 1);
        assert_eq!(state.extra["call_2"], json!({ "a": 1 }));

        assert!(ReplayState::read(&json!("text")).is_empty());
        assert!(ReplayState::read(&Value::Null).is_empty());
    }
}
