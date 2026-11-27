//! LLM provider adapters: the OpenAI-compatible Chat Completions and Anthropic Messages wire
//! protocols over `reqwest`, with SSE parsing by `sse-stream`. [`Gateway`] implements the
//! `LlmGateway` port of `copilot-core`, which roadmap step P1.3 froze.
//!
//! Status: OpenAI-compatible connections stream text, reasoning (`reasoning`,
//! `reasoning_content`, OpenRouter `reasoning_details`), tool calls, and spoken replies; send
//! tools, tool results, images, PDFs, and `input_audio`; replay provider state per dialect; and
//! list models with what the provider reports. Anthropic connections stream text, adaptive
//! thinking (summarized), and tool calls; send tools, tool results, images, and PDFs with cache
//! breakpoints; replay the answer's content blocks (thinking signatures and `redacted_thinking`
//! included); report refusals with their `stop_details`; and list models with their reported
//! capabilities. A part the protocol cannot carry fails with `PROVIDER_UNSUPPORTED` before
//! anything is sent.
//!
//! Rules (see "Adapter Rules" in `docs/design/copilot.md`):
//!
//! - Send every request through the client that `copilot_core::Outbound::pick` returns for the
//!   target's base URL and network scope, from the `Outbound<reqwest::Client>` the composition
//!   root injects (built by `copilot-security`). Never build a client here.
//! - Cap the size of each SSE event (`copilot_core::cap_events`) and of each whole body
//!   (`copilot_core::read_capped`); `sse-stream` does not bound an event in progress.
//! - Log a request the outbound policy refused (`copilot_core::Blocked::find`) as
//!   `provider.http.blocked`, other failures as `provider.http.failed`. Never echo a response
//!   body or a resolved address to the client, and never log prompts, answers, or keys.
//! - No provider types leave this crate: map them to the core port types.
//! - No `axum`, `sqlx`, `utoipa`, contracts, database, middleware, other adapter, or rustauth
//!   crates (`make boundaries`).

mod anthropic;
mod gateway;
mod http;
mod openai;
mod schema;
mod sse;

pub use crate::gateway::{Gateway, UNSUPPORTED};
