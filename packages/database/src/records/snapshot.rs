//! Turn snapshot records: `turn_contexts.tools` and `turn_contexts.files`.

use copilot_core::{FileId, SnapshotFile, SnapshotTool, ToolAlias, ToolServerId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::records::{RecordError, VERSION, check, hex, parse, unhex};

/// A tool a turn offered: alias, server, original name, description, and input schema.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SnapshotToolRecord {
    pub v: u8,
    pub alias: String,
    /// `null` for a built-in tool.
    pub server_id: Option<Uuid>,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// A file of the turn's preamble and how it went to the model.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SnapshotFileRecord {
    pub v: u8,
    pub file_id: Uuid,
    /// Lowercase hex; all zeros for a deleted file.
    pub sha256: String,
    /// `inline`, `native`, `placeholder`, `in_project`, `deleted`, or `too_large`.
    pub rendering: String,
}

impl From<&SnapshotTool> for SnapshotToolRecord {
    fn from(tool: &SnapshotTool) -> Self {
        Self {
            v: VERSION,
            alias: tool.alias.as_str().to_owned(),
            server_id: tool.server_id.map(ToolServerId::uuid),
            name: tool.name.clone(),
            description: tool.description.clone(),
            input_schema: tool.input_schema.clone(),
        }
    }
}

impl TryFrom<SnapshotToolRecord> for SnapshotTool {
    type Error = RecordError;

    fn try_from(record: SnapshotToolRecord) -> Result<Self, Self::Error> {
        check(record.v)?;
        Ok(Self {
            alias: ToolAlias::restore(record.alias),
            server_id: record.server_id.map(ToolServerId::new),
            name: record.name,
            description: record.description,
            input_schema: record.input_schema,
        })
    }
}

impl From<&SnapshotFile> for SnapshotFileRecord {
    fn from(file: &SnapshotFile) -> Self {
        Self {
            v: VERSION,
            file_id: file.file_id.uuid(),
            sha256: hex(&file.sha256),
            rendering: file.rendering.as_str().to_owned(),
        }
    }
}

impl TryFrom<SnapshotFileRecord> for SnapshotFile {
    type Error = RecordError;

    fn try_from(record: SnapshotFileRecord) -> Result<Self, Self::Error> {
        check(record.v)?;
        Ok(Self {
            file_id: FileId::new(record.file_id),
            sha256: unhex(&record.sha256)?,
            rendering: parse(&record.rendering)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use copilot_core::Rendering;
    use serde_json::json;

    use super::*;

    #[test]
    fn tools_and_files_round_trip() {
        let tool = SnapshotTool {
            alias: ToolAlias::restore("docs__search".to_owned()),
            server_id: Some(ToolServerId::generate()),
            name: "search".to_owned(),
            description: "Search the docs.".to_owned(),
            input_schema: json!({"type": "object"}),
        };
        let json = serde_json::to_value(SnapshotToolRecord::from(&tool)).unwrap();
        let record: SnapshotToolRecord = serde_json::from_value(json).unwrap();
        assert_eq!(SnapshotTool::try_from(record).unwrap(), tool);

        let file = SnapshotFile {
            file_id: FileId::generate(),
            sha256: [0x0f; 32],
            rendering: Rendering::InProject,
        };
        let json = serde_json::to_value(SnapshotFileRecord::from(&file)).unwrap();
        assert_eq!(json["rendering"], "in_project");
        assert_eq!(json["sha256"], "0f".repeat(32));
        let record: SnapshotFileRecord = serde_json::from_value(json).unwrap();
        assert_eq!(SnapshotFile::try_from(record).unwrap(), file);
    }
}
