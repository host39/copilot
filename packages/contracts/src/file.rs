//! Files: project files and chat attachments (`/projects/{id}/files`, `/chats/{id}/files`,
//! `/files/{id}`). Bytes come from `GET /files/{id}/content`, never inside JSON.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// The multipart field that carries an upload's bytes (see [`UploadFileForm`]).
pub const UPLOAD_FIELD: &str = "file";

/// A stored file. Exactly one of `projectId` and `chatId` is set.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct FileResponse {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub chat_id: Option<Uuid>,
    /// Display only, never a path.
    pub name: String,
    /// From the allowlist, checked against the bytes.
    pub media_type: String,
    pub size_bytes: u64,
    /// Rough token size for the capacity bar.
    pub token_estimate: u32,
    /// PDFs only.
    pub page_count: Option<u32>,
    /// Project files only: sent to every chat of the project while `true`.
    pub included: bool,
    pub created_at: DateTime<Utc>,
}

/// `POST /projects/{id}/files` and `POST /chats/{id}/files`: a `multipart/form-data` body with
/// one part named `file`, whose file name becomes the display name. The answer is the stored
/// [`FileResponse`].
#[derive(Clone, Debug, ToSchema)]
pub struct UploadFileForm {
    #[schema(value_type = String, format = Binary)]
    pub file: Vec<u8>,
}

/// `PATCH /files/{id}`: only the given fields change.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct UpdateFileRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Project files only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub included: Option<bool>,
}

/// `POST /files/{id}/move`: moves a chat attachment or a project file into a project, without a
/// copy.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct MoveFileRequest {
    pub project_id: Uuid,
}

/// How full a project's context is with its included files.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum CapacityLevel {
    /// No default model, or its context window is unknown.
    Unknown,
    Fits,
    /// Past 80%: uploads warn.
    Warn,
    /// Past 100%: sending is blocked.
    Over,
}

/// The capacity bar: summed token estimates of the included files against the default model's
/// context window.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct Capacity {
    pub used: u64,
    pub window: Option<u32>,
    pub level: CapacityLevel,
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn file_sends_both_owner_fields() {
        let file = FileResponse {
            id: Uuid::nil(),
            project_id: None,
            chat_id: Some(Uuid::nil()),
            name: "notes.md".to_owned(),
            media_type: "text/markdown".to_owned(),
            size_bytes: 1200,
            token_estimate: 300,
            page_count: None,
            included: true,
            created_at: Utc.with_ymd_and_hms(2026, 11, 26, 12, 0, 0).unwrap(),
        };
        let json = serde_json::to_value(&file).expect("serializes");
        assert_eq!(json["projectId"], serde_json::Value::Null);
        assert_eq!(json["chatId"], "00000000-0000-0000-0000-000000000000");
        assert_eq!(json["pageCount"], serde_json::Value::Null);
        assert_eq!(json["sizeBytes"], 1200);
    }

    #[test]
    fn capacity_level_is_snake_case() {
        let capacity = Capacity {
            used: 9000,
            window: Some(10_000),
            level: CapacityLevel::Warn,
        };
        assert_eq!(
            serde_json::to_value(capacity).expect("serializes"),
            serde_json::json!({ "used": 9000, "window": 10000, "level": "warn" })
        );
    }
}
