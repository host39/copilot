//! `/settings`: the caller's own settings. Account, password, and devices are rustauth's
//! `/auth/*` routes.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// `GET` and `PATCH /settings`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SettingsResponse {
    /// Personal instructions, sent with every chat. Empty until the first change.
    pub instructions: String,
}

/// `PATCH /settings`: only the given fields change.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct UpdateSettingsRequest {
    /// Surrounding whitespace is trimmed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_patch_changes_nothing() {
        let request: UpdateSettingsRequest =
            serde_json::from_value(serde_json::json!({})).expect("valid body");
        assert_eq!(request, UpdateSettingsRequest::default());
    }
}
