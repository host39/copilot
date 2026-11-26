use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// `GET /`: API info.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct RootResponse {
    pub status: String,
    pub environment: String,
}

/// `GET /health`: liveness check.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub status: String,
}

/// `GET /health/ready`: readiness check, including database connectivity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ReadyResponse {
    pub status: String,
    pub database: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_health_as_flat_json() {
        let body = HealthResponse {
            status: "healthy".to_owned(),
        };
        assert_eq!(
            serde_json::to_value(body).expect("serializes"),
            serde_json::json!({ "status": "healthy" })
        );
    }
}
