//! The context hash: SHA-256 over canonical JSON (`docs/design/copilot.md`, Context Building,
//! step 7). Canonical means object keys sorted at every level and no insignificant whitespace,
//! so the hash never depends on map order or on serde features.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::domains::chat::{ContextHash, SnapshotFile, SnapshotTool};
use crate::domains::provider::Capabilities;
use crate::shared::ids::MessageId;
use crate::shared::reasoning::ReasoningLevel;

/// Everything a turn's requests depend on besides the messages themselves.
pub(crate) struct HashInput<'a> {
    pub(crate) system: &'a str,
    pub(crate) tools: &'a [SnapshotTool],
    /// The preamble and every file part of the history in the window, in request order.
    pub(crate) files: &'a [SnapshotFile],
    pub(crate) capabilities: &'a Capabilities,
    pub(crate) reasoning: Option<ReasoningLevel>,
    pub(crate) window_start_id: Option<MessageId>,
}

pub(crate) fn hash(input: &HashInput<'_>) -> ContextHash {
    let capabilities = input.capabilities;
    let value = json!({
        "system": input.system,
        "tools": input
            .tools
            .iter()
            .map(|tool| json!([tool.alias.as_str(), tool.description, tool.input_schema]))
            .collect::<Vec<_>>(),
        "files": input
            .files
            .iter()
            .map(|file| json!([file.file_id.to_string(), hex(&file.sha256), file.rendering.as_str()]))
            .collect::<Vec<_>>(),
        "render_profile": {
            "vision": capabilities.vision,
            "pdf": capabilities.pdf,
            "tools": capabilities.tools,
            "audio_in": capabilities.audio_in,
            "audio_out": capabilities.audio_out,
            "reasoning": capabilities.reasoning,
            "reasoning_level": input.reasoning.map(ReasoningLevel::as_str),
        },
        "window_start_id": input.window_start_id.map(|id| id.to_string()),
    });
    let mut text = String::new();
    canonical(&value, &mut text);
    ContextHash::new(Sha256::digest(text.as_bytes()).into())
}

/// Writes `value` as canonical JSON.
pub(crate) fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (position, key) in keys.into_iter().enumerate() {
                if position > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                canonical(&map[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (position, item) in items.iter().enumerate() {
                if position > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::chat::Rendering;
    use crate::domains::tool::ToolAlias;
    use crate::shared::ids::FileId;

    fn tool(schema: Value) -> SnapshotTool {
        SnapshotTool {
            alias: ToolAlias::builtin("list_schedules"),
            server_id: None,
            name: "list_schedules".to_owned(),
            description: "List.".to_owned(),
            input_schema: schema,
        }
    }

    #[test]
    fn sorts_keys_at_every_level_without_whitespace() {
        let mut out = String::new();
        canonical(
            &serde_json::from_str(r#"{"b": [1, {"z": "x", "a": null}], "a": "é\n"}"#).unwrap(),
            &mut out,
        );
        assert_eq!(out, r#"{"a":"é\n","b":[1,{"a":null,"z":"x"}]}"#);
    }

    fn input<'a>(
        tools: &'a [SnapshotTool],
        files: &'a [SnapshotFile],
        capabilities: &'a Capabilities,
    ) -> HashInput<'a> {
        HashInput {
            system: "Be brief.",
            tools,
            files,
            capabilities,
            reasoning: None,
            window_start_id: None,
        }
    }

    #[test]
    fn the_hash_ignores_key_order_and_follows_every_input() {
        let capabilities = Capabilities::default();
        let files = [SnapshotFile {
            file_id: FileId::generate(),
            sha256: [3; 32],
            rendering: Rendering::Inline,
        }];
        let first = [tool(
            serde_json::from_str(r#"{"type":"object","properties":{}}"#).unwrap(),
        )];
        let second = [tool(
            serde_json::from_str(r#"{"properties":{},"type":"object"}"#).unwrap(),
        )];
        let base = hash(&input(&first, &files, &capabilities));
        assert_eq!(base, hash(&input(&second, &files, &capabilities)));
        let vision = Capabilities {
            vision: true,
            ..Capabilities::default()
        };
        let changes = [
            HashInput {
                system: "Be long.",
                ..input(&first, &files, &capabilities)
            },
            HashInput {
                reasoning: Some(ReasoningLevel::High),
                ..input(&first, &files, &capabilities)
            },
            HashInput {
                window_start_id: Some(MessageId::generate()),
                ..input(&first, &files, &capabilities)
            },
            input(&first, &[], &capabilities),
            input(&[], &files, &capabilities),
            input(&first, &files, &vision),
        ];
        for changed in changes {
            assert_ne!(hash(&changed), base);
        }
    }
}
