//! Helpers for colocated route tests: the app on a lazy pool and lib-rustauth's in-memory fakes,
//! so route tests need no Postgres.

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use copilot_core::{Environment, FileLimits, LogLevel, PrivateNetwork};
use copilot_database::{DbConfig, DbPool, DbSsl};
use copilot_security::SecretKey;
use futures_util::StreamExt;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use crate::config::AppConfig;
use crate::helpers::auth::Identity;
use crate::helpers::auth::testing::{SETUP_TOKEN, config as identity_config, identity};
use crate::helpers::bootstrap::Adapters;
use crate::{App, AppDependencies, assemble, create_app};

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

/// The whole app on a throwaway database (`#[sqlx::test]`): migrated, rustauth on Postgres, and
/// the real provider and MCP adapters on the hardened clients.
pub async fn live_app(db: DbPool) -> Router {
    live_app_with(db, config()).await
}

/// [`live_app`] with another config (for example smaller file limits).
pub async fn live_app_with(db: DbPool, config: AppConfig) -> Router {
    live(db, config).await.router
}

/// [`live_app`] with its shared state, for tests that drain the runs or recover a restart. Each
/// call is a new process on the same database: its own run registry and shutdown token.
pub async fn live(db: DbPool, config: AppConfig) -> App {
    copilot_database::operations::migrate::run(&db)
        .await
        .expect("migrated");
    let identity = Identity::start(db.clone(), &identity_config())
        .await
        .expect("rustauth on Postgres");
    assemble(AppDependencies {
        config,
        db,
        identity,
        adapters: Adapters::create().expect("the outbound clients build"),
        shutdown: CancellationToken::new(),
    })
}

pub struct TestResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

pub async fn send(app: Router, request: Request<Body>) -> TestResponse {
    let (status, headers, bytes) = read(app, request).await;
    TestResponse {
        status,
        headers,
        body: serde_json::from_slice(bytes.as_bytes()).unwrap_or(Value::Null),
    }
}

/// The status, headers, and whole body as text (an event stream ends when its run is done).
pub async fn read(app: Router, request: Request<Body>) -> (StatusCode, HeaderMap, String) {
    let response = app.oneshot(request).await.expect("infallible router");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let text = String::from_utf8(bytes.to_vec()).expect("a UTF-8 body");
    (status, headers, text)
}

/// A server-sent event stream read one event at a time; dropping it disconnects.
pub struct Events {
    body: axum::body::BodyDataStream,
    buffer: String,
}

impl Events {
    /// The `data` of the next event, `None` once the stream ended. Fails after 10 seconds.
    pub async fn next(&mut self) -> Option<Value> {
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let frame: String = self.buffer.drain(..end + 2).collect();
                let data: Vec<&str> = frame
                    .lines()
                    .filter_map(|line| line.strip_prefix("data: "))
                    .collect();
                if data.is_empty() {
                    continue; // a keep-alive comment
                }
                return Some(serde_json::from_str(&data.join("\n")).expect("JSON event data"));
            }
            let chunk = tokio::time::timeout(Duration::from_secs(10), self.body.next())
                .await
                .expect("an event within 10 seconds")?
                .expect("a body chunk");
            self.buffer
                .push_str(std::str::from_utf8(&chunk).expect("UTF-8 events"));
        }
    }

    /// Events up to and including the first of type `kind`.
    pub async fn until(&mut self, kind: &str) -> Vec<Value> {
        let mut events = Vec::new();
        while let Some(event) = self.next().await {
            let found = event["type"] == kind;
            events.push(event);
            if found {
                return events;
            }
        }
        panic!("the stream ended before a {kind} event: {events:?}");
    }

    /// Every event left, up to the end of the stream.
    pub async fn rest(&mut self) -> Vec<Value> {
        let mut events = Vec::new();
        while let Some(event) = self.next().await {
            events.push(event);
        }
        events
    }
}

/// Sends `request` and opens its answer as an event stream (any other answer: the status and
/// the body read as JSON, in the `Err`).
pub async fn open(app: Router, request: Request<Body>) -> Result<Events, Box<TestResponse>> {
    let response = app.oneshot(request).await.expect("infallible router");
    let status = response.status();
    let headers = response.headers().clone();
    let is_stream = headers
        .get("content-type")
        .is_some_and(|value| value == "text/event-stream");
    if status == StatusCode::OK && is_stream {
        return Ok(Events {
            body: response.into_body().into_data_stream(),
            buffer: String::new(),
        });
    }
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    Err(Box::new(TestResponse {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }))
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

/// A second account (role `user`), created by the administrator whose token is `admin` through
/// rustauth's admin route, and its session token.
pub async fn other_user(app: &Router, admin: &str, username: &str) -> String {
    let password = "another horse battery staple";
    let body = json!({
        "username": username,
        "displayName": username,
        "temporaryPassword": password,
        "role": "user",
    });
    let created = send(
        app.clone(),
        request(Method::POST, "/api/admin/users", Some(admin), Some(body)),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let body = json!({ "username": username, "password": password });
    let response = send(
        app.clone(),
        request(Method::POST, "/api/auth/login", None, Some(body)),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    response.body["data"]["token"]
        .as_str()
        .expect("a session token")
        .to_owned()
}

/// A provider that lists the model `mock-1` (the chat completions route is the caller's).
pub async fn provider() -> wiremock::MockServer {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [{ "id": "mock-1", "object": "model", "created": 1, "owned_by": "test" }],
        })))
        .mount(&server)
        .await;
    server
}

/// A connection of `token`'s user to `server` (loopback: the user must be the administrator),
/// refreshed; the id of its enabled model.
pub async fn model(app: &Router, token: &str, server: &wiremock::MockServer) -> String {
    model_at(app, token, &format!("{}/v1", server.uri())).await
}

/// [`model`] for a provider at `base_url` (ending in `/v1`).
pub async fn model_at(app: &Router, token: &str, base_url: &str) -> String {
    let created = send(
        app.clone(),
        request(
            Method::POST,
            "/api/providers",
            Some(token),
            Some(json!({
                "kind": "openai",
                "preset": "custom",
                "name": "Mock",
                "baseUrl": base_url,
            })),
        ),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let provider = created.body["data"]["id"].as_str().expect("an id");
    let refreshed = send(
        app.clone(),
        request(
            Method::POST,
            &format!("/api/providers/{provider}/refresh"),
            Some(token),
            None,
        ),
    )
    .await;
    assert_eq!(refreshed.status, StatusCode::OK, "{}", refreshed.body);
    let models = send(
        app.clone(),
        request(
            Method::GET,
            &format!("/api/providers/{provider}/models"),
            Some(token),
            None,
        ),
    )
    .await;
    models.body["items"][0]["id"]
        .as_str()
        .expect("a model")
        .to_owned()
}

/// A tool server of `user` written with SQL, without credentials or tools; its id. Its host is
/// under `.invalid`, so a turn that lists it fails at once and never reaches the network.
pub async fn tool_server(db: &DbPool, user: &str, slug: &str) -> String {
    let id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO tool_servers (created_at, updated_at, id, user_id, name, slug, url) \
         SELECT now(), now(), $1, id, 'Docs', $2, 'https://mcp.example.invalid' \
         FROM auth.users WHERE username = $3",
    )
    .bind(id)
    .bind(slug)
    .bind(user)
    .execute(db)
    .await
    .expect("tool server inserted");
    id.to_string()
}

/// A `multipart/form-data` request with one part per `(field, file name, bytes)`.
pub fn multipart(uri: &str, token: &str, parts: &[(&str, &str, &[u8])]) -> Request<Body> {
    const BOUNDARY: &str = "copilot-test-boundary";
    let mut body = Vec::new();
    for (field, file_name, bytes) in parts {
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{field}\"; \
                 filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(body))
        .expect("request")
}
