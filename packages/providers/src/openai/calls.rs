//! Tool calls reassembled from streamed fragments (`choices[].delta.tool_calls[]`).
//!
//! The first fragment of a call usually carries `index`, `id`, `type`, and `function.name`;
//! later fragments carry `function.arguments` pieces under the same `index`. Some servers send
//! each call whole in one fragment (Ollama), leave `index` out (some Gemini responses), or
//! number every whole call `0`. So a fragment continues a call when its `index` matches and its
//! `id`, if any, does not contradict the call's; without an `index`, when its `id` matches, or,
//! without either, the last call. Anything else starts a new call.

use serde_json::{Map, Value};

/// One call of the answer.
#[derive(Debug)]
pub(crate) struct Call {
    /// The adapter's part index of the call.
    pub(crate) part: u32,
    /// The provider's `index`, when it sent one.
    wire: Option<u64>,
    /// The provider's call id.
    pub(crate) id: Option<String>,
    pub(crate) ended: bool,
    /// Fields other than `index`, `id`, `type`, and `function` (Gemini thought signatures).
    pub(crate) extra: Map<String, Value>,
}

/// One fragment as read from the wire.
#[derive(Debug, Default)]
pub(crate) struct Fragment {
    pub(crate) wire: Option<u64>,
    pub(crate) id: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) arguments: String,
    pub(crate) extra: Map<String, Value>,
}

/// Tool call fields every fragment may carry; the rest are extra, and stored extra fields never
/// replace these on replay.
pub(crate) const CALL_FIELDS: [&str; 4] = ["index", "id", "type", "function"];

impl Fragment {
    /// Reads one `tool_calls[]` item; `None` for anything that is not an object.
    pub(crate) fn read(item: Value) -> Option<Self> {
        let Value::Object(mut fields) = item else {
            return None;
        };
        let wire = fields.get("index").and_then(Value::as_u64);
        let id = fields
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_owned);
        let function = fields.remove("function");
        let name = function
            .as_ref()
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        // Arguments are a JSON string; a server that sends an object gets it serialized.
        let arguments = match function
            .as_ref()
            .and_then(|function| function.get("arguments"))
        {
            Some(Value::String(text)) => text.clone(),
            None | Some(Value::Null) => String::new(),
            Some(other) => other.to_string(),
        };
        let extra = fields
            .into_iter()
            .filter(|(key, _)| !CALL_FIELDS.contains(&key.as_str()))
            .collect();
        Some(Self {
            wire,
            id,
            name,
            arguments,
            extra,
        })
    }
}

/// The calls of one answer, in the order they started.
#[derive(Debug, Default)]
pub(crate) struct Calls {
    list: Vec<Call>,
}

impl Calls {
    /// The call `fragment` continues, if any (see the module docs).
    pub(crate) fn find(&self, fragment: &Fragment) -> Option<usize> {
        match (fragment.wire, fragment.id.as_deref()) {
            (Some(wire), id) => {
                let position = self.list.iter().rposition(|call| call.wire == Some(wire))?;
                let known = self.list[position].id.as_deref();
                match (id, known) {
                    (Some(id), Some(known)) if id != known => None,
                    _ => Some(position),
                }
            }
            (None, Some(id)) => self
                .list
                .iter()
                .position(|call| call.id.as_deref() == Some(id)),
            (None, None) => self.list.len().checked_sub(1),
        }
    }

    /// Starts a call as part `part`.
    pub(crate) fn start(&mut self, part: u32, fragment: &Fragment) -> usize {
        self.list.push(Call {
            part,
            wire: fragment.wire,
            id: fragment.id.clone(),
            ended: false,
            extra: Map::new(),
        });
        self.list.len() - 1
    }

    pub(crate) fn get_mut(&mut self, position: usize) -> &mut Call {
        &mut self.list[position]
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Calls not ended yet, marked ended.
    pub(crate) fn end(&mut self) -> Vec<u32> {
        self.list
            .iter_mut()
            .filter(|call| !call.ended)
            .map(|call| {
                call.ended = true;
                call.part
            })
            .collect()
    }

    /// The extra fields of every call with an id and extra fields, by id.
    pub(crate) fn extra(&self) -> Map<String, Value> {
        self.list
            .iter()
            .filter(|call| !call.extra.is_empty())
            .filter_map(|call| {
                let id = call.id.clone()?;
                Some((id, Value::Object(call.extra.clone())))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn fragment(value: Value) -> Fragment {
        Fragment::read(value).expect("an object")
    }

    #[test]
    fn reads_a_fragment_leniently() {
        let read = fragment(json!({
            "index": 1,
            "id": "call_1",
            "type": "function",
            "function": { "name": "files__read", "arguments": "{\"pa" },
            "extra_content": { "google": { "thought_signature": "sig" } },
        }));
        assert_eq!(read.wire, Some(1));
        assert_eq!(read.id.as_deref(), Some("call_1"));
        assert_eq!(read.name.as_deref(), Some("files__read"));
        assert_eq!(read.arguments, "{\"pa");
        assert_eq!(
            Value::Object(read.extra),
            json!({ "extra_content": { "google": { "thought_signature": "sig" } } })
        );

        let object_arguments =
            fragment(json!({ "id": "", "function": { "arguments": { "path": "a" } } }));
        assert_eq!(object_arguments.id, None, "an empty id is no id");
        assert_eq!(object_arguments.arguments, "{\"path\":\"a\"}");
        assert!(Fragment::read(json!("not a call")).is_none());
    }

    #[test]
    fn continues_calls_by_index_id_or_position() {
        let mut calls = Calls::default();
        let first = fragment(json!({ "index": 0, "id": "call_a" }));
        assert_eq!(calls.find(&first), None);
        calls.start(0, &first);
        assert_eq!(calls.find(&fragment(json!({ "index": 0 }))), Some(0));
        assert_eq!(
            calls.find(&fragment(json!({ "index": 0, "id": "call_a" }))),
            Some(0),
            "a repeated id continues the call"
        );
        assert_eq!(
            calls.find(&fragment(json!({ "index": 0, "id": "call_b" }))),
            None,
            "another id under the same index is a new call"
        );
        assert_eq!(calls.find(&fragment(json!({ "index": 1 }))), None);

        let mut unnumbered = Calls::default();
        let whole = fragment(json!({ "id": "call_x" }));
        unnumbered.start(3, &whole);
        assert_eq!(
            unnumbered.find(&fragment(json!({ "id": "call_x" }))),
            Some(0)
        );
        assert_eq!(unnumbered.find(&fragment(json!({ "id": "call_y" }))), None);
        assert_eq!(
            unnumbered.find(&fragment(json!({ "function": { "arguments": "}" } }))),
            Some(0),
            "no index and no id continue the last call"
        );
        assert_eq!(Calls::default().find(&fragment(json!({}))), None);
    }

    #[test]
    fn ends_each_call_once_and_keys_extra_fields_by_id() {
        let mut calls = Calls::default();
        let with_extra = fragment(json!({ "index": 0, "id": "call_a" }));
        let position = calls.start(4, &with_extra);
        calls
            .get_mut(position)
            .extra
            .insert("extra_content".to_owned(), json!({ "sig": 1 }));
        calls.start(5, &fragment(json!({ "index": 1, "id": "call_b" })));
        let anonymous = calls.start(6, &fragment(json!({ "index": 2 })));
        calls
            .get_mut(anonymous)
            .extra
            .insert("lost".to_owned(), json!(true));

        assert_eq!(calls.end(), [4, 5, 6]);
        assert!(calls.end().is_empty());
        assert_eq!(
            Value::Object(calls.extra()),
            json!({ "call_a": { "extra_content": { "sig": 1 } } }),
            "a call without an id cannot be matched on replay"
        );
    }
}
