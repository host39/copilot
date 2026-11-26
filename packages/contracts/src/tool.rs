//! `/tool-servers`: remote MCP servers. Credentials (a bearer token or custom headers) go in and
//! never come out; responses say whether they are stored.

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::shared::{KeyState, redacted};

/// A server's default for its tools.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    /// Every call waits for the user's decision. The default for new servers.
    #[default]
    Ask,
    /// Calls run without asking.
    Auto,
}

/// One tool's setting; a tool without one follows the server's [`ApprovalMode`].
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ToolMode {
    /// Never sent to the model.
    Off,
    Ask,
    Auto,
}

/// A tool from the server's last listing.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ServerTool {
    /// The original name; per-tool modes and approvals use it.
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    /// The tool's JSON Schema, as the server sent it.
    pub input_schema: Value,
    /// The server's `readOnlyHint`: the UI suggests `auto`, nothing more (annotations are
    /// untrusted).
    pub read_only: bool,
}

/// A tool server (lists, details, and writes).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ToolServerResponse {
    pub id: Uuid,
    pub name: String,
    /// The prefix of the server's tool names sent to models (`{slug}__{tool}`); never changes.
    pub slug: String,
    pub url: String,
    pub approval: ApprovalMode,
    /// Per-tool modes by original tool name.
    pub tool_modes: BTreeMap<String, ToolMode>,
    pub enabled: bool,
    /// Whether a bearer token or custom headers are stored. Lists report `set` or `missing`;
    /// details report `unreadable` after a `SECRET_KEY` change.
    pub credentials: KeyState,
    /// The negotiated MCP revision.
    pub protocol_version: Option<String>,
    /// The cached tool list; `null` until a Test succeeds.
    pub tools: Option<Vec<ServerTool>>,
    pub tools_fetched_at: Option<DateTime<Utc>>,
    /// The tool list changed (or the URL did) since the last review: every call asks until
    /// `POST /tool-servers/{id}/review`.
    pub review_required: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `POST /tool-servers`.
#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct CreateToolServerRequest {
    pub name: String,
    /// 1 to 12 lowercase letters or digits; derived from the name when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    pub url: String,
    /// Sent as `Authorization: Bearer <token>`. Blank or absent means none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer: Option<String>,
    /// Custom headers. `Authorization` is rejected when `bearer` is set, and MCP's own headers
    /// always.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Default `ask`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalMode>,
}

impl fmt::Debug for CreateToolServerRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateToolServerRequest")
            .field("name", &self.name)
            .field("slug", &self.slug)
            .field("url", &self.url)
            .field("bearer", &redacted(self.bearer.as_ref()))
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .field("approval", &self.approval)
            .finish()
    }
}

/// New credentials for a server: they replace the stored bearer token and headers together.
#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("bearer", &redacted(self.bearer.as_ref()))
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// `PATCH /tool-servers/{id}`: only the given fields change. A URL with another origin clears
/// the stored credentials unless `credentials` comes with it, and puts every tool back to `ask`
/// until the server is reviewed.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct UpdateToolServerRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// New credentials; `null` removes the stored ones.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::patch::nullable"
    )]
    pub credentials: Option<Option<Credentials>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalMode>,
    /// Replaces every per-tool mode. Names must be on the server's tool list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_modes: Option<BTreeMap<String, ToolMode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// Tool names that changed between two listings of a server.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ToolChanges {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// Same name, new description or input schema.
    pub changed: Vec<String>,
}

/// `POST /tool-servers/{id}/test`: connects, lists the tools, and refreshes the cache. Any
/// change sets `reviewRequired`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ToolTestResponse {
    pub server: ToolServerResponse,
    pub changes: ToolChanges,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_tells_keeping_clearing_and_replacing_credentials_apart() {
        let keep: UpdateToolServerRequest =
            serde_json::from_value(serde_json::json!({ "name": "Docs" })).expect("valid body");
        assert_eq!(keep.credentials, None);
        let clear: UpdateToolServerRequest =
            serde_json::from_value(serde_json::json!({ "credentials": null })).expect("valid body");
        assert_eq!(clear.credentials, Some(None));
        let replace: UpdateToolServerRequest = serde_json::from_value(serde_json::json!({
            "credentials": { "headers": { "X-Api-Key": "secret" } },
        }))
        .expect("valid body");
        let credentials = replace.credentials.flatten().expect("credentials");
        assert_eq!(credentials.bearer, None);
        assert_eq!(credentials.headers["X-Api-Key"], "secret");
    }

    #[test]
    fn modes_are_snake_case_and_keyed_by_tool_name() {
        let request: UpdateToolServerRequest = serde_json::from_value(serde_json::json!({
            "approval": "auto",
            "toolModes": { "search_docs": "off", "read_page": "auto" },
        }))
        .expect("valid body");
        assert_eq!(request.approval, Some(ApprovalMode::Auto));
        let modes = request.tool_modes.expect("modes");
        assert_eq!(modes["search_docs"], ToolMode::Off);
        assert_eq!(modes["read_page"], ToolMode::Auto);
    }

    #[test]
    fn create_defaults_to_no_credentials() {
        let request: CreateToolServerRequest = serde_json::from_value(serde_json::json!({
            "name": "Docs",
            "url": "https://mcp.example.com/mcp",
        }))
        .expect("valid body");
        assert_eq!(request.bearer, None);
        assert!(request.headers.is_empty());
        assert_eq!(request.approval, None);
    }

    #[test]
    fn debug_hides_tokens_and_header_values() {
        let headers = BTreeMap::from([("X-Api-Key".to_owned(), "header-secret".to_owned())]);
        let create = CreateToolServerRequest {
            name: "Docs".to_owned(),
            url: "https://mcp.example.com/mcp".to_owned(),
            bearer: Some("token-secret".to_owned()),
            headers: headers.clone(),
            ..CreateToolServerRequest::default()
        };
        let update = UpdateToolServerRequest {
            credentials: Some(Some(Credentials {
                bearer: Some("token-secret".to_owned()),
                headers,
            })),
            ..UpdateToolServerRequest::default()
        };
        let printed = format!("{create:?} {update:?}");
        assert!(!printed.contains("secret"), "{printed}");
        assert!(printed.contains("X-Api-Key"), "{printed}");
    }
}
