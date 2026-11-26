//! LLM provider adapters: the OpenAI-compatible Chat Completions and Anthropic Messages wire
//! protocols over `reqwest`, with SSE parsing by `sse-stream`. They implement the `LlmGateway` port
//! of `copilot-core`, which roadmap step P1.3 freezes; Phase 2 track B fills this crate in.
//!
//! Rules (see "Adapter Rules" in `docs/design/copilot.md`):
//!
//! - Send every request through the client that `copilot_core::Outbound::pick` returns for the
//!   target's base URL and network scope, from the `Outbound<reqwest::Client>` the composition
//!   root injects (built by `copilot-security`). Never build a client here.
//! - Cap the size of each SSE event (`copilot_core::cap_events`) and of each whole body
//!   (`copilot_core::read_capped`); `sse-stream` does not bound an event in progress.
//! - Log a request the outbound policy refused (`copilot_core::Blocked::find`) as
//!   `provider.http.blocked`, other failures as `provider.http.failed`.
//! - No provider types leave this crate: map them to the core port types.
//! - No `axum`, `sqlx`, `utoipa`, contracts, database, middleware, other adapter, or rustauth
//!   crates (`make boundaries`).
