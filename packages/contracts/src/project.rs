//! `/projects`: reusable context (instructions, files, default model, reasoning, tools) for
//! many chats.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::file::Capacity;
use crate::shared::{BuiltinTool, ReasoningLevel};

/// A project (lists, writes, and the base of [`ProjectDetailsResponse`]).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ProjectResponse {
    pub id: Uuid,
    pub name: String,
    /// One emoji, or empty.
    pub icon: String,
    /// The project's system prompt.
    pub instructions: String,
    pub default_model_id: Option<Uuid>,
    /// `null`: the model's default.
    pub reasoning: Option<ReasoningLevel>,
    pub builtin_tools: Vec<BuiltinTool>,
    /// Tool servers every chat of the project uses (a chat can add or remove servers).
    pub tool_server_ids: Vec<Uuid>,
    pub pinned: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `GET /projects/{id}`: the project plus its capacity bar.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ProjectDetailsResponse {
    #[serde(flatten)]
    pub project: ProjectResponse,
    pub capacity: Capacity,
}

/// Query of `GET /projects`, most recently updated first.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, IntoParams, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct ProjectListQuery {
    /// Only pinned (`true`) or only unpinned (`false`) projects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
    /// At most this many (the sidebar shows the 5 most recent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// `POST /projects`. Every field but `name` has a default (empty, none, or off).
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin_tools: Option<Vec<BuiltinTool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_server_ids: Option<Vec<Uuid>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
}

/// `PATCH /projects/{id}`: only the given fields change; `null` clears the nullable ones.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct UpdateProjectRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::patch::nullable"
    )]
    pub default_model_id: Option<Option<Uuid>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::patch::nullable"
    )]
    pub reasoning: Option<Option<ReasoningLevel>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin_tools: Option<Vec<BuiltinTool>>,
    /// Replaces the project's tool servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_server_ids: Option<Vec<Uuid>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_needs_only_a_name() {
        let request: CreateProjectRequest =
            serde_json::from_value(serde_json::json!({ "name": "Research" })).expect("valid body");
        assert_eq!(
            request,
            CreateProjectRequest {
                name: "Research".to_owned(),
                ..CreateProjectRequest::default()
            }
        );
        let missing = serde_json::from_value::<CreateProjectRequest>(serde_json::json!({}));
        assert!(missing.is_err());
    }

    #[test]
    fn update_clears_the_default_model_with_null() {
        let request: UpdateProjectRequest = serde_json::from_value(serde_json::json!({
            "defaultModelId": null,
            "reasoning": "high",
            "toolServerIds": [],
        }))
        .expect("valid body");
        assert_eq!(request.default_model_id, Some(None));
        assert_eq!(request.reasoning, Some(Some(ReasoningLevel::High)));
        assert_eq!(request.tool_server_ids, Some(Vec::new()));
        assert_eq!(request.name, None);
    }

    #[test]
    fn list_query_reads_camel_case() {
        let query: ProjectListQuery =
            serde_json::from_value(serde_json::json!({ "pinned": true, "limit": 5 }))
                .expect("valid query");
        assert_eq!(query.pinned, Some(true));
        assert_eq!(query.limit, Some(5));
    }
}
