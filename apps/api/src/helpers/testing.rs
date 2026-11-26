//! Helpers for colocated route tests.

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use copilot_core::{Environment, LogLevel};
use copilot_database::{DbConfig, DbPool, DbSsl};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use crate::config::AppConfig;
use crate::{AppDependencies, create_app};

/// Local config whose database is unreachable (port 1), so readiness reports `down`.
pub fn config() -> AppConfig {
    AppConfig {
        environment: Environment::Local,
        host: "127.0.0.1".to_owned(),
        port: 3000,
        log_level: LogLevel::Error,
        cors_allowed_origins: Vec::new(),
        db: DbConfig {
            host: "127.0.0.1".to_owned(),
            port: 1,
            database: "test".to_owned(),
            username: "postgres".to_owned(),
            password: "postgres".to_owned(),
            ssl: DbSsl::Disabled,
            pool_max: 1,
            connection_timeout: Duration::from_millis(500),
        },
    }
}

pub fn app_with(config: AppConfig) -> Router {
    let db = copilot_database::create_pool(&config.db);
    create_app(AppDependencies { config, db })
}

pub fn app() -> Router {
    app_with(config())
}

/// The app on an existing pool, for `#[sqlx::test]` tests that get a throwaway database.
pub fn app_on(db: DbPool) -> Router {
    create_app(AppDependencies {
        config: config(),
        db,
    })
}

pub struct TestResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

pub async fn send(app: Router, request: Request<Body>) -> TestResponse {
    let response = app.oneshot(request).await.expect("infallible router");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    TestResponse {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

pub async fn get(app: Router, uri: &str) -> TestResponse {
    let request = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request");
    send(app, request).await
}
