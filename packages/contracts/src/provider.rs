//! `/providers`: model provider connections. API keys go in and never come out; responses say
//! whether one is stored.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::shared::{KeyState, redacted};

/// The wire protocol of a connection.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// Chat Completions, `POST {base}/chat/completions`.
    OpenAi,
    /// Messages API, `POST {base}/v1/messages`.
    Anthropic,
}

/// Defaults for a connection: base URL, dialect, and capability rules. `anthropic` and `custom`
/// fit the `anthropic` kind; every preset except `anthropic` fits the `openai` kind.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum Preset {
    OpenAi,
    Anthropic,
    Gemini,
    OpenRouter,
    Ollama,
    /// Any other compatible server or gateway.
    Custom,
}

/// A provider connection (list items, and the base of [`ProviderDetailsResponse`]).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ProviderResponse {
    pub id: Uuid,
    pub kind: ProviderKind,
    pub preset: Preset,
    pub name: String,
    pub base_url: String,
    pub enabled: bool,
    /// Lists report `set` or `missing` without opening the key; `GET /providers/{id}` reports
    /// `unreadable` when it was sealed with another `SECRET_KEY`.
    pub key: KeyState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// What deleting a connection would affect: the chats, projects, and schedules that use one of
/// its models. Schedules among them are turned off ("Model removed").
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsage {
    pub chats: u64,
    pub projects: u64,
    pub schedules: u64,
}

/// `GET /providers/{id}`: the connection plus `usage`, for the delete confirmation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ProviderDetailsResponse {
    #[serde(flatten)]
    pub provider: ProviderResponse,
    pub usage: ProviderUsage,
}

/// `POST /providers`.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct CreateProviderRequest {
    pub kind: ProviderKind,
    pub preset: Preset,
    pub name: String,
    /// Default: the preset's base URL (`custom` needs one). `https`, or `http` for a private host
    /// the server allows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Sealed at rest and never returned. Blank or absent means no key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

impl fmt::Debug for CreateProviderRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateProviderRequest")
            .field("kind", &self.kind)
            .field("preset", &self.preset)
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("api_key", &redacted(self.api_key.as_ref()))
            .finish()
    }
}

/// `PATCH /providers/{id}`: only the given fields change. A base URL with another origin
/// (scheme, host, or port) clears the stored key unless `apiKey` comes with it.
#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct UpdateProviderRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// A new key; an empty string clears the stored one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

impl fmt::Debug for UpdateProviderRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpdateProviderRequest")
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("api_key", &redacted(self.api_key.as_ref()))
            .field("enabled", &self.enabled)
            .finish()
    }
}

/// `POST /providers/{id}/refresh`: what listing the provider's models changed. A refresh never
/// deletes a model; one the provider stopped listing becomes unavailable.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct RefreshResponse {
    /// New models.
    pub added: u32,
    /// Known models the provider still lists.
    pub updated: u32,
    /// Known models the provider no longer lists.
    pub unavailable: u32,
    /// New models enabled automatically (when the provider lists 20 or fewer).
    pub enabled: u32,
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn provider() -> ProviderResponse {
        let at = Utc.with_ymd_and_hms(2026, 11, 26, 12, 0, 0).unwrap();
        ProviderResponse {
            id: Uuid::nil(),
            kind: ProviderKind::OpenAi,
            preset: Preset::OpenRouter,
            name: "OpenRouter".to_owned(),
            base_url: "https://openrouter.ai/api/v1".to_owned(),
            enabled: true,
            key: KeyState::Set,
            created_at: at,
            updated_at: at,
        }
    }

    #[test]
    fn details_flatten_the_provider_beside_usage() {
        let details = ProviderDetailsResponse {
            provider: provider(),
            usage: ProviderUsage {
                chats: 3,
                projects: 1,
                schedules: 0,
            },
        };
        assert_eq!(
            serde_json::to_value(&details).expect("serializes"),
            serde_json::json!({
                "id": "00000000-0000-0000-0000-000000000000",
                "kind": "openai",
                "preset": "openrouter",
                "name": "OpenRouter",
                "baseUrl": "https://openrouter.ai/api/v1",
                "enabled": true,
                "key": "set",
                "createdAt": "2026-11-26T12:00:00Z",
                "updatedAt": "2026-11-26T12:00:00Z",
                "usage": { "chats": 3, "projects": 1, "schedules": 0 },
            })
        );
        let read: ProviderDetailsResponse =
            serde_json::from_value(serde_json::to_value(&details).expect("serializes"))
                .expect("round trip");
        assert_eq!(read, details);
    }

    #[test]
    fn create_reads_camel_case_and_optional_fields() {
        let request: CreateProviderRequest = serde_json::from_value(serde_json::json!({
            "kind": "anthropic",
            "preset": "anthropic",
            "name": "Claude",
            "apiKey": "sk-ant-secret",
        }))
        .expect("valid body");
        assert_eq!(request.kind, ProviderKind::Anthropic);
        assert_eq!(request.base_url, None);
        assert_eq!(request.api_key.as_deref(), Some("sk-ant-secret"));
    }

    #[test]
    fn debug_hides_api_keys() {
        let create = CreateProviderRequest {
            kind: ProviderKind::OpenAi,
            preset: Preset::OpenAi,
            name: "OpenAI".to_owned(),
            base_url: None,
            api_key: Some("sk-secret".to_owned()),
        };
        let update = UpdateProviderRequest {
            api_key: Some("sk-secret".to_owned()),
            ..UpdateProviderRequest::default()
        };
        let printed = format!("{create:?} {update:?}");
        assert!(!printed.contains("sk-secret"), "{printed}");
        assert!(printed.contains("***"), "{printed}");
    }

    #[test]
    fn empty_patch_serializes_empty() {
        assert_eq!(
            serde_json::to_value(UpdateProviderRequest::default()).expect("serializes"),
            serde_json::json!({})
        );
    }
}
