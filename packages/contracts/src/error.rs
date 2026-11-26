use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

/// Body of every failure response (`4xx` and `5xx`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct ErrorResponse {
    /// Human-readable message.
    pub error: String,
    /// Machine-readable code, for example `VALIDATION_ERROR`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// The request field that failed validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub details: Option<BTreeMap<String, Value>>,
    /// Echo of the `X-Request-Id` header, for support and log correlation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl ErrorResponse {
    pub fn new(error: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            code: Some(code.into()),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omits_empty_fields() {
        let body = ErrorResponse::new("Not found", "NOT_FOUND");
        assert_eq!(
            serde_json::to_value(body).expect("serializes"),
            serde_json::json!({ "error": "Not found", "code": "NOT_FOUND" })
        );
    }
}
