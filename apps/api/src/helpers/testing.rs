//! Helpers for colocated route tests: the app on a lazy pool and lib-rustauth's in-memory fakes,
//! so route tests need no Postgres.

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use copilot_core::{Environment, FileLimits, LogLevel, PrivateNetwork};
use copilot_database::{DbConfig, DbPool, DbSsl};
use copilot_security::SecretKey;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use crate::config::AppConfig;
use crate::helpers::auth::Identity;
use crate::helpers::auth::testing::{SETUP_TOKEN, config as identity_config, identity};
use crate::helpers::bootstrap::Adapters;
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
        secret_key: SecretKey::new(&[7; 32]),
        private_network: PrivateNetwork::Admins,
        files: FileLimits::default(),
        web_dir: None,
        identity: identity_config(),
    }
}

/// The app on `db` with `config`, rustauth on fakes, and the unavailable adapters.
pub async fn app_with(config: AppConfig, db: DbPool) -> Router {
    app_on_identity(config, db, identity().await)
}

/// The app with a given rustauth (for example one started on a migrated test database).
pub fn app_on_identity(config: AppConfig, db: DbPool, identity: Identity) -> Router {
    create_app(AppDependencies {
        config,
        db,
        identity,
        adapters: Adapters::unavailable(),
        shutdown: CancellationToken::new(),
    })
}

pub async fn app() -> Router {
    let config = config();
    let db = copilot_database::create_pool(&config.db);
    app_with(config, db).await
}

/// The app on an existing pool, for `#[sqlx::test]` tests that get a throwaway database.
pub async fn app_on(db: DbPool) -> Router {
    app_with(config(), db).await
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

/// A request with an optional bearer token and JSON body.
pub fn request(
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => builder.body(Body::empty()),
    }
    .expect("request")
}

/// Registers the first account (the administrator) through rustauth's route and returns its
/// session token.
pub async fn register(app: &Router, username: &str) -> String {
    let body = json!({
        "username": username,
        "password": "correct horse battery staple",
        "displayName": username,
        "setupToken": SETUP_TOKEN,
    });
    let response = send(
        app.clone(),
        request(Method::POST, "/api/auth/register", None, Some(body)),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    response.body["data"]["token"]
        .as_str()
        .expect("a session token")
        .to_owned()
}
