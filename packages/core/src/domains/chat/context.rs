//! Turn snapshots: the system text, tools, and files a turn was built with.
//!
//! `conversation::context::build` (P1.3) computes the hash and the snapshot. Every request of a
//! turn, including the continuation after an approval, is built from the saved snapshot, so
//! settings changes apply from the next user message.

use std::fmt;

use serde_json::Value;

use crate::domains::tool::ToolAlias;
use crate::shared::ids::{FileId, MessageId, ToolServerId};

/// SHA-256 over the canonical JSON of a turn's request inputs (`docs/design/copilot.md`,
/// Context Building, step 7).
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct ContextHash([u8; 32]);

impl ContextHash {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// A stored hash (`bytea`); `None` unless exactly 32 bytes.
    pub fn restore(bytes: &[u8]) -> Option<Self> {
        bytes.try_into().ok().map(Self)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for ContextHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hex: String = self.0.iter().map(|byte| format!("{byte:02x}")).collect();
        write!(formatter, "ContextHash({hex})")
    }
}

/// A tool as a turn offered it.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotTool {
    pub alias: ToolAlias,
    /// `None` for a built-in tool.
    pub server_id: Option<ToolServerId>,
    /// The original name.
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// A file as a turn rendered it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotFile {
    pub file_id: FileId,
    pub sha256: [u8; 32],
    /// How the file went to the model (for example `inline`, `native`, `placeholder`).
    pub rendering: String,
}

/// A turn's snapshot (`turn_contexts`), keyed by chat and hash.
#[derive(Clone, Debug, PartialEq)]
pub struct TurnContext {
    pub hash: ContextHash,
    pub system: String,
    /// Sorted by alias.
    pub tools: Vec<SnapshotTool>,
    pub files: Vec<SnapshotFile>,
    /// The first message in the window; `None` when the window starts at the first message.
    pub window_start_id: Option<MessageId>,
}

impl TurnContext {
    /// The snapshot entry for `alias`; a call to an alias that is not here is rejected.
    pub fn tool(&self, alias: &ToolAlias) -> Option<&SnapshotTool> {
        self.tools.iter().find(|tool| &tool.alias == alias)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn restores_only_32_byte_hashes() {
        let hash = ContextHash::new([7; 32]);
        assert_eq!(ContextHash::restore(hash.as_bytes()), Some(hash));
        assert_eq!(ContextHash::restore(&[1, 2, 3]), None);
        assert!(format!("{hash:?}").starts_with("ContextHash(0707"));
    }

    #[test]
    fn finds_tools_by_alias() {
        let alias = ToolAlias::builtin("list_schedules");
        let context = TurnContext {
            hash: ContextHash::new([0; 32]),
            system: String::new(),
            tools: vec![SnapshotTool {
                alias: alias.clone(),
                server_id: None,
                name: "list_schedules".to_owned(),
                description: String::new(),
                input_schema: json!({"type": "object"}),
            }],
            files: Vec::new(),
            window_start_id: None,
        };
        assert!(context.tool(&alias).is_some());
        assert!(
            context
                .tool(&ToolAlias::builtin("cancel_schedule"))
                .is_none()
        );
    }
}
