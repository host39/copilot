//! A scriptable fake model server for manual testing and the smoke test (`scripts/smoke.sh`).
//!
//! It speaks both wire formats Copilot's adapters use, on one port:
//!
//! - **OpenAI-compatible** (base URL `http://127.0.0.1:4010/v1`): `GET /v1/models`,
//!   `POST /v1/chat/completions`, and the Responses API's `POST /v1/responses` (stateless, as
//!   with `store: false`), streamed or not, with `Authorization: Bearer <key>`.
//! - **Anthropic** (base URL `http://127.0.0.1:4010`): `GET /v1/models` and `POST /v1/messages`,
//!   streamed or not, with `x-api-key: <key>` and `anthropic-version`.
//! - **MCP** at `/mcp` (Streamable HTTP, both protocol eras) with the tools `echo` and `add`.
//!
//! Keywords in the last user message pick the reply (`tools/fake-llm/README.md`). Dev only: no
//! Copilot crate depends on it, and the image never contains it.

pub mod anthropic;
mod config;
mod http;
pub mod mcp;
pub mod openai;
pub mod reply;
pub mod responses;
pub mod script;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};

pub use crate::config::{Config, ConfigError, DEFAULT_KEY, DEFAULT_PORT};

/// The largest request body: requests carry images, PDFs, and audio clips inline as base64.
const BODY_MAX_BYTES: usize = 64 * 1_024 * 1_024;

/// A listed model.
#[derive(Clone, Copy, Debug)]
pub struct Model {
    pub id: &'static str,
    pub name: &'static str,
    /// Reports reasoning (OpenRouter's `supported_parameters`, Anthropic's thinking types).
    pub reasoning: bool,
    /// Reports audio input and output (OpenRouter's modalities; Anthropic has no audio).
    pub audio: bool,
}

/// Every model both formats list.
pub const MODELS: [Model; 3] = [
    Model {
        id: "fake-chat",
        name: "Fake Chat",
        reasoning: false,
        audio: false,
    },
    Model {
        id: "fake-reasoner",
        name: "Fake Reasoner",
        reasoning: true,
        audio: false,
    },
    Model {
        id: "fake-audio",
        name: "Fake Audio",
        reasoning: false,
        audio: true,
    },
];

/// The context window every model reports.
pub const CONTEXT_WINDOW: u32 = 128_000;
/// The output limit every model reports.
pub const MAX_OUTPUT_TOKENS: u32 = 8_192;

/// A listed model by id.
pub fn model(id: &str) -> Option<Model> {
    MODELS.into_iter().find(|model| model.id == id)
}

#[derive(Clone, Debug)]
pub struct AppState {
    config: Arc<Config>,
    /// Numbers requests: response ids, call ids, and request id headers.
    sequence: Arc<AtomicU64>,
}

impl AppState {
    fn next(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::Relaxed) + 1
    }
}

/// The whole server: the model routes, `/mcp`, and `/health`.
pub fn app(config: Config) -> Router {
    let mcp = mcp::router(config.mcp_token.clone());
    let state = AppState {
        config: Arc::new(config),
        sequence: Arc::new(AtomicU64::new(0)),
    };
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(openai::complete))
        .route("/v1/responses", post(responses::respond))
        .route("/v1/messages", post(anthropic::messages))
        .with_state(state)
        .merge(mcp)
        .fallback(|| async { (StatusCode::NOT_FOUND, "fake-llm: no such route") })
        .layer(DefaultBodyLimit::max(BODY_MAX_BYTES))
}

/// `GET /v1/models`: Anthropic's list when the request carries Anthropic's headers, else the
/// OpenAI-compatible one.
async fn models(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if headers.contains_key(anthropic::VERSION) || headers.contains_key(anthropic::API_KEY) {
        anthropic::models(&state, &headers).into_response()
    } else {
        openai::models(&state, &headers).into_response()
    }
}
