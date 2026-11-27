//! Requests against the whole router, without a socket.

#![allow(dead_code)]

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use fake_llm::{Config, app};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

pub const KEY: &str = "test-key";

/// The server with no pauses between chunks.
pub fn server() -> Router {
    app(Config {
        key: KEY.to_owned(),
        chunk_ms: 0,
        slow_ms: 0,
        ..Config::default()
    })
}

pub struct Answer {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: String,
}

impl Answer {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).expect("a JSON body")
    }

    /// The SSE events as `(event name, data)` pairs.
    pub fn events(&self) -> Vec<(Option<String>, String)> {
        let mut out = Vec::new();
        for block in self.body.split("\n\n") {
            let mut name = None;
            let mut data = Vec::new();
            for line in block.lines() {
                if let Some(value) = line.strip_prefix("event:") {
                    name = Some(value.trim().to_owned());
                } else if let Some(value) = line.strip_prefix("data:") {
                    data.push(value.strip_prefix(' ').unwrap_or(value).to_owned());
                }
            }
            if !data.is_empty() {
                out.push((name, data.join("\n")));
            }
        }
        out
    }

    /// The data of every event that is JSON.
    pub fn chunks(&self) -> Vec<Value> {
        self.events()
            .iter()
            .filter_map(|(_, data)| serde_json::from_str(data).ok())
            .collect()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }
}

pub async fn send(request: Request<Body>) -> Answer {
    let response = server().oneshot(request).await.expect("infallible");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("a body")
        .to_bytes();
    Answer {
        status,
        headers,
        body: String::from_utf8(bytes.to_vec()).expect("UTF-8"),
    }
}

/// `POST /v1/chat/completions` with the key.
pub async fn openai(body: Value) -> Answer {
    send(
        Request::post("/v1/chat/completions")
            .header("authorization", format!("Bearer {KEY}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("a request"),
    )
    .await
}

/// `POST /v1/messages` with the key and version.
pub async fn anthropic(body: Value) -> Answer {
    send(
        Request::post("/v1/messages")
            .header("x-api-key", KEY)
            .header("anthropic-version", "2026-10-21")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("a request"),
    )
    .await
}

/// `POST /v1/responses` with the key.
pub async fn responses(body: Value) -> Answer {
    send(
        Request::post("/v1/responses")
            .header("authorization", format!("Bearer {KEY}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("a request"),
    )
    .await
}
