//! MCP client over Streamable HTTP through the official `rmcp` SDK, for both the 2026-07-28
//! revision and older 2025 servers. It implements the `ToolHost` port of `copilot-core`, which
//! roadmap step P1.3 freezes; Phase 2 track C fills this crate in.
//!
//! Rules (see "Adapter Rules" in `docs/design/copilot.md` and `docs/research/mcp.md`):
//!
//! - Hand rmcp the `reqwest::Client` the composition root injects (built by `copilot-security`).
//!   Never build a client here, and never enable rmcp's `reqwest` feature, which turns on
//!   reqwest's aws-lc-rs provider.
//! - Time out every call and cap every response (`MCP_RESPONSE_MAX_BYTES`).
//! - No rmcp types leave this crate, so an rmcp major bump touches only this crate.
//! - No `axum`, `sqlx`, `utoipa`, contracts, database, middleware, other adapter, or rustauth
//!   crates (`make boundaries`).
