//! Health routes at the root, for probes: liveness and readiness (database connectivity). They
//! are public; the web bundle owns `/`.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use copilot_contracts::{HealthResponse, ReadyResponse};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(health))
        .routes(routes!(ready))
}

#[utoipa::path(
    get,
    path = "/health",
    operation_id = "health",
    tag = "health",
    responses((status = OK, description = "Liveness check", body = HealthResponse))
)]
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_owned(),
    })
}

#[utoipa::path(
    get,
    path = "/health/ready",
    operation_id = "health_ready",
    tag = "health",
    responses(
        (status = OK, description = "Readiness check: database connectivity", body = ReadyResponse),
        (status = SERVICE_UNAVAILABLE, description = "Service unavailable", body = ReadyResponse)
    )
)]
pub async fn ready(State(state): State<AppState>) -> (StatusCode, Json<ReadyResponse>) {
    if copilot_database::ping(&state.db).await {
        let body = ReadyResponse {
            status: "healthy".to_owned(),
            database: "up".to_owned(),
        };
        (StatusCode::OK, Json(body))
    } else {
        let body = ReadyResponse {
            status: "unhealthy".to_owned(),
            database: "down".to_owned(),
        };
        (StatusCode::SERVICE_UNAVAILABLE, Json(body))
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use copilot_database::DbPool;
    use copilot_middleware::REQUEST_ID_HEADER;
    use serde_json::json;

    use crate::helpers::testing::{app, app_on, get, send};

    #[tokio::test]
    async fn health_returns_healthy_status() {
        let response = get(app().await, "/health").await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, json!({ "status": "healthy" }));
    }

    #[tokio::test]
    async fn ready_returns_503_when_the_database_is_unreachable() {
        let response = get(app().await, "/health/ready").await;
        assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response.body,
            json!({ "status": "unhealthy", "database": "down" })
        );
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run `make test-db`"]
    async fn ready_returns_healthy_when_the_database_is_reachable(pool: DbPool) {
        let response = get(app_on(pool).await, "/health/ready").await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(
            response.body,
            json!({ "status": "healthy", "database": "up" })
        );
    }

    #[tokio::test]
    async fn echoes_the_incoming_request_id() {
        let request = Request::builder()
            .uri("/health")
            .header(REQUEST_ID_HEADER, "ray-health")
            .body(Body::empty())
            .expect("request");
        let response = send(app().await, request).await;
        assert_eq!(response.headers[REQUEST_ID_HEADER], "ray-health");
    }

    #[tokio::test]
    async fn unknown_routes_return_the_error_envelope() {
        let response = get(app().await, "/missing").await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        assert_eq!(response.body["error"], "Not found");
        assert_eq!(response.body["code"], "NOT_FOUND");
        assert!(response.body["requestId"].is_string());
    }
}
