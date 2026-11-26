//! Values that several route families share.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// What `Debug` prints instead of a secret (an API key, a bearer token, a header value).
pub(crate) const REDACTED: &str = "***";

/// How hard a reasoning model thinks. Where a level is optional, `null` means the next default:
/// a chat inherits its project's level, a project or schedule the model's own default.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ReasoningLevel {
    Off,
    Low,
    Medium,
    High,
}

/// A tool set built into Copilot, enabled per project or per chat.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum BuiltinTool {
    /// `schedule_prompt`, `list_schedules`, and `cancel_schedule`; offered to interactive runs
    /// only.
    Schedules,
}

/// Whether a sealed secret (a provider's API key, a tool server's credentials) is stored.
/// Responses never carry the secret itself.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum KeyState {
    Missing,
    Set,
    /// Stored, but sealed with another `SECRET_KEY`: it must be entered again.
    Unreadable,
}

/// `Debug` text for an optional secret: whether it is there, never its value.
pub(crate) fn redacted(secret: Option<&String>) -> Option<&'static str> {
    secret.map(|_| REDACTED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_snake_case() {
        assert_eq!(
            serde_json::to_value([ReasoningLevel::Off, ReasoningLevel::High]).expect("serializes"),
            serde_json::json!(["off", "high"])
        );
        assert_eq!(
            serde_json::to_value(BuiltinTool::Schedules).expect("serializes"),
            "schedules"
        );
        assert_eq!(
            serde_json::to_value(KeyState::Unreadable).expect("serializes"),
            "unreadable"
        );
    }

    #[test]
    fn redacts_present_secrets_only() {
        assert_eq!(redacted(Some(&"sk-secret".to_owned())), Some(REDACTED));
        assert_eq!(redacted(None), None);
    }
}
