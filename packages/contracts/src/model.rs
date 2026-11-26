//! `/models` and `/providers/{id}/models`: the models of the caller's connections.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::provider::{Preset, ProviderKind};
use crate::shared::ReasoningLevel;

/// A capability flag. The names match the fields of [`Capabilities`].
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub enum Capability {
    Vision,
    Pdf,
    Tools,
    AudioIn,
    AudioOut,
    Reasoning,
}

/// What a model can take and return.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub vision: bool,
    pub pdf: bool,
    pub tools: bool,
    pub audio_in: bool,
    pub audio_out: bool,
    pub reasoning: bool,
    /// Levels the picker offers; empty unless `reasoning`.
    pub reasoning_levels: Vec<ReasoningLevel>,
    /// The model's own default level, when the provider reports one.
    pub default_reasoning: Option<ReasoningLevel>,
}

/// The user's capability overrides; `null` keeps the detected flag. Refresh never changes them.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(default, rename_all = "camelCase")]
pub struct CapabilityOverrides {
    pub vision: Option<bool>,
    pub pdf: Option<bool>,
    pub tools: Option<bool>,
    pub audio_in: Option<bool>,
    pub audio_out: Option<bool>,
    pub reasoning: Option<bool>,
}

/// A flag the UI shows without a toggle, and why (for example `audioIn` on Claude models).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct CapabilityLock {
    pub capability: Capability,
    pub reason: String,
}

/// A model of one of the caller's connections.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ModelResponse {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub provider_name: String,
    pub kind: ProviderKind,
    pub preset: Preset,
    /// The provider's model id.
    pub remote_id: String,
    pub display_name: String,
    pub enabled: bool,
    /// `false` once a refresh no longer lists the model.
    pub available: bool,
    /// Reported by the provider or set by the user; `null` when unknown.
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    /// What requests use: detected flags, then overrides, then locks.
    pub capabilities: Capabilities,
    /// What the last refresh detected, before overrides and locks.
    pub detected: Capabilities,
    pub overrides: CapabilityOverrides,
    pub locks: Vec<CapabilityLock>,
    /// The `openai` preset turns reasoning off whenever tools are in the request, and the
    /// picker says so.
    pub reasoning_off_with_tools: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `PATCH /models/{id}`: only the given fields change. Disabling a model turns off the schedules
/// that use it ("Model turned off").
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct UpdateModelRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Replaces every override. Overriding a locked flag is rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_overrides: Option<CapabilityOverrides>,
    /// `null` clears it (unknown until a refresh reports one).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::patch::nullable"
    )]
    pub context_window: Option<Option<u32>>,
    /// `null` clears it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::patch::nullable"
    )]
    pub max_output_tokens: Option<Option<u32>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_names_match_the_capability_fields() {
        let flags = [
            Capability::Vision,
            Capability::Pdf,
            Capability::Tools,
            Capability::AudioIn,
            Capability::AudioOut,
            Capability::Reasoning,
        ];
        let capabilities = serde_json::to_value(Capabilities {
            vision: false,
            pdf: false,
            tools: false,
            audio_in: false,
            audio_out: false,
            reasoning: false,
            reasoning_levels: Vec::new(),
            default_reasoning: None,
        })
        .expect("serializes");
        for flag in flags {
            let name = serde_json::to_value(flag).expect("serializes");
            let name = name.as_str().expect("a string");
            assert!(capabilities.get(name).is_some(), "{name}");
        }
        assert_eq!(
            serde_json::to_value(Capability::AudioIn).expect("serializes"),
            "audioIn"
        );
    }

    #[test]
    fn overrides_read_partial_objects() {
        let overrides: CapabilityOverrides =
            serde_json::from_value(serde_json::json!({ "vision": true })).expect("valid body");
        assert_eq!(
            overrides,
            CapabilityOverrides {
                vision: Some(true),
                ..CapabilityOverrides::default()
            }
        );
    }

    #[test]
    fn update_clears_limits_with_null_and_keeps_them_when_absent() {
        let clear: UpdateModelRequest =
            serde_json::from_value(serde_json::json!({ "contextWindow": null }))
                .expect("valid body");
        assert_eq!(clear.context_window, Some(None));
        assert_eq!(clear.max_output_tokens, None);
        let set: UpdateModelRequest =
            serde_json::from_value(serde_json::json!({ "maxOutputTokens": 4096 }))
                .expect("valid body");
        assert_eq!(set.max_output_tokens, Some(Some(4096)));
        assert_eq!(
            serde_json::to_value(&clear).expect("serializes"),
            serde_json::json!({ "contextWindow": null })
        );
    }
}
