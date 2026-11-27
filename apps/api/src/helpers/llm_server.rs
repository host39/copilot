//! An in-process OpenAI-compatible provider for route tests that need a run to stay live: each
//! chat completion streams what the test writes, chunk by chunk, until the test finishes or
//! drops its [`Writer`]. It listens on loopback (`http://127.0.0.1:<port>/v1`, which only a
//! private-scope connection may reach) and lists one model, `mock-1`. wiremock answers with a
//! whole body at once, so the tests that stop, reattach to, or disconnect from a run halfway use
//! this server instead.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::stream;
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// The provider. Dropping it leaves the server running until the test's runtime ends.
pub struct LlmServer {
    /// The base URL to connect with (`.../v1`).
    pub url: String,
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    /// The streams of the next completions, in order.
    answers: Mutex<VecDeque<mpsc::UnboundedReceiver<String>>>,
    /// Every completion request body, in order.
    requests: Mutex<Vec<Value>>,
}

impl Shared {
    fn answers(&self) -> MutexGuard<'_, VecDeque<mpsc::UnboundedReceiver<String>>> {
        self.answers.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl LlmServer {
    pub async fn start() -> Self {
        let shared = Arc::new(Shared::default());
        let router = axum::Router::new()
            .route("/v1/models", get(models))
            .route("/v1/chat/completions", post(completion))
            .with_state(Arc::clone(&shared));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port");
        let address = listener.local_addr().expect("an address");
        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("the provider serves");
        });
        Self {
            url: format!("http://{address}/v1"),
            shared,
        }
    }

    /// The next completion streams what the returned writer sends.
    pub fn live(&self) -> Writer {
        let (sender, receiver) = mpsc::unbounded_channel();
        self.shared.answers().push_back(receiver);
        Writer { sender }
    }

    /// The next completion answers `pieces` as text deltas, then finishes with `stop`.
    pub fn reply(&self, pieces: &[&str]) {
        let writer = self.live();
        for piece in pieces {
            writer.text(piece);
        }
        writer.finish("stop");
    }

    /// The bodies of the completion requests so far.
    pub fn requests(&self) -> Vec<Value> {
        self.shared
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Writes one completion's stream.
pub struct Writer {
    sender: mpsc::UnboundedSender<String>,
}

impl Writer {
    pub fn text(&self, text: &str) {
        self.chunk(json!({ "content": text }), Value::Null);
    }

    /// Ends the answer with `reason` (`stop`, `length`, ...) and closes the stream.
    pub fn finish(self, reason: &str) {
        self.chunk(json!({}), json!(reason));
        let _ = self.sender.send("data: [DONE]\n\n".to_owned());
    }

    /// One tool call of `alias` with `arguments`, at `index`.
    pub fn call(&self, index: u32, alias: &str, arguments: &str) {
        self.chunk(
            json!({ "tool_calls": [{
                "index": index,
                "id": format!("call_{index}"),
                "type": "function",
                "function": { "name": alias, "arguments": arguments },
            }] }),
            Value::Null,
        );
    }

    fn chunk(&self, delta: Value, finish: Value) {
        let chunk = json!({
            "id": "chatcmpl-1",
            "object": "chat.completion.chunk",
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
        });
        // The answer may have ended already (the run was stopped); nothing to do then.
        let _ = self.sender.send(format!("data: {chunk}\n\n"));
    }
}

async fn models() -> Json<Value> {
    Json(json!({
        "object": "list",
        "data": [{ "id": "mock-1", "object": "model", "created": 1, "owned_by": "test" }],
    }))
}

async fn completion(State(shared): State<Arc<Shared>>, Json(body): Json<Value>) -> Response {
    shared
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(body);
    let Some(receiver) = shared.answers().pop_front() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no answer scripted").into_response();
    };
    let chunks = stream::unfold(receiver, |mut receiver| async move {
        let chunk = receiver.recv().await?;
        Some((Ok::<_, Infallible>(Bytes::from(chunk)), receiver))
    });
    Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(chunks))
        .expect("a response")
}
