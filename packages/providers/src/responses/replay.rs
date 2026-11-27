//! The provider-opaque state of a Responses answer (`provider_state.content`, see "Stored JSON
//! Shapes" in `docs/design/copilot.md`): the answer's output items in order, as the API finished
//! them, under `{"api": "responses", "items": [...]}`.
//!
//! The Responses API is used statelessly (`store: false`), so the next request carries the
//! answer's items again: `reasoning` items with their `encrypted_content`, `message` items (their
//! `phase` included), and `function_call` items. Only these three types are kept; Copilot asks
//! for no built-in tools, so nothing else should appear. A reasoning item without
//! `encrypted_content` cannot be sent back statelessly (the API would look it up by id), so it is
//! dropped, and then the other items lose their `id` too: an item id that points at a reasoning
//! item the request does not carry is refused.
//!
//! The decoder writes the state once per answer, just before `Finished`; the request builder
//! reads it back when core replays the answer to the same connection. Reading is strict about
//! the marker and lenient about the rest: a state written by the Chat Completions adapter (after
//! the connection switched APIs) reads as absent, and the answer is rebuilt from its parts.

use serde_json::{Map, Value, json};

/// The marker of a Responses state.
const API: &str = "responses";
/// The output item types an answer replays.
const KEPT: [&str; 3] = ["message", "reasoning", "function_call"];

/// The stored state of `items` (finished output items in output order), or `None` when none is
/// worth keeping.
pub(crate) fn state(items: Vec<Value>) -> Option<Value> {
    let mut kept: Vec<Map<String, Value>> = items
        .into_iter()
        .filter_map(|item| match item {
            Value::Object(item) if kept_type(&item) => Some(item),
            _ => None,
        })
        .collect();
    let before = kept.len();
    kept.retain(|item| !is_reasoning(item) || has_encrypted(item));
    if kept.len() < before {
        for item in &mut kept {
            item.remove("id");
        }
    }
    if kept.is_empty() {
        return None;
    }
    Some(json!({ "api": API, "items": kept }))
}

/// The items of a stored state, or `None` when `value` is not a Responses state (another API's
/// state, or nothing readable). Items of other types are skipped.
pub(crate) fn items(value: &Value) -> Option<Vec<Value>> {
    let state = value.as_object()?;
    if state.get("api").and_then(Value::as_str) != Some(API) {
        return None;
    }
    let items: Vec<Value> = state
        .get("items")?
        .as_array()?
        .iter()
        .filter(|item| item.as_object().is_some_and(kept_type))
        .cloned()
        .collect();
    (!items.is_empty()).then_some(items)
}

/// The `call_id` of a `function_call` item.
pub(crate) fn call_id(item: &Value) -> Option<&str> {
    (item.get("type").and_then(Value::as_str) == Some("function_call"))
        .then(|| item.get("call_id").and_then(Value::as_str))
        .flatten()
}

fn kept_type(item: &Map<String, Value>) -> bool {
    item.get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| KEPT.contains(&kind))
}

fn is_reasoning(item: &Map<String, Value>) -> bool {
    item.get("type").and_then(Value::as_str) == Some("reasoning")
}

fn has_encrypted(item: &Map<String, Value>) -> bool {
    item.get("encrypted_content")
        .and_then(Value::as_str)
        .is_some_and(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reasoning(encrypted: Option<&str>) -> Value {
        let mut item = json!({
            "id": "rs_1",
            "type": "reasoning",
            "summary": [{ "type": "summary_text", "text": "Thinking." }],
        });
        if let Some(encrypted) = encrypted {
            item["encrypted_content"] = Value::from(encrypted);
        }
        item
    }

    fn call() -> Value {
        json!({
            "id": "fc_1",
            "type": "function_call",
            "call_id": "call_1",
            "name": "docs__search",
            "arguments": "{\"q\":\"x\"}",
            "status": "completed",
        })
    }

    #[test]
    fn keeps_replayable_items_in_order_under_the_marker() {
        let message = json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "phase": "final_answer",
            "content": [{ "type": "output_text", "text": "Hi", "annotations": [] }],
        });
        let other = json!({ "id": "ws_1", "type": "web_search_call", "status": "completed" });
        let state = state(vec![
            reasoning(Some("gAAA")),
            other,
            call(),
            message.clone(),
            Value::from("junk"),
        ])
        .unwrap();
        assert_eq!(
            state,
            json!({ "api": "responses", "items": [reasoning(Some("gAAA")), call(), message] })
        );
        assert_eq!(items(&state).unwrap().len(), 3);
        assert_eq!(call_id(&call()), Some("call_1"));
        assert_eq!(call_id(&reasoning(None)), None);
    }

    #[test]
    fn reasoning_without_encrypted_content_is_dropped_with_every_id() {
        let state = state(vec![reasoning(None), call()]).unwrap();
        let items = items(&state).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "function_call");
        assert!(
            items[0].get("id").is_none(),
            "no id points at the dropped item"
        );
        assert_eq!(items[0]["call_id"], "call_1");
        assert_eq!(
            super::state(vec![reasoning(Some(""))]),
            None,
            "nothing left to keep"
        );
    }

    #[test]
    fn reads_only_its_own_state() {
        let chat = json!({ "reasoning_key": "reasoning", "reasoning": "Hmm" });
        assert_eq!(items(&chat), None, "a Chat Completions state");
        assert_eq!(items(&json!({ "api": "responses", "items": [] })), None);
        assert_eq!(items(&json!({ "api": "responses", "items": "x" })), None);
        assert_eq!(items(&json!([1, 2])), None);
        let mixed = json!({ "api": "responses", "items": [call(), { "type": "image" }, 3] });
        assert_eq!(items(&mixed), Some(vec![call()]));
    }
}
