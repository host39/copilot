//! MCP client over Streamable HTTP through the official `rmcp` SDK, for both the stateless
//! 2026-07-28 revision and older servers with `initialize` and `Mcp-Session-Id` sessions
//! (2025-11-25 and earlier). [`McpHost`] implements the `ToolHost` port of `copilot-core`, which
//! roadmap step P1.3 froze.
//!
//! Behavior (`docs/design/copilot.md`, Tools and Outbound Requests; `docs/research/mcp.md`):
//!
//! - **Eras.** A server with no known revision is probed with `server/discover` and falls back
//!   to `initialize` when it turns out to be legacy (rmcp's `Auto` lifecycle). The revision a
//!   connection negotiated is remembered per server and URL for the life of the process, and
//!   core stores it with each tool list (`ToolEndpoint::protocol_version`), so later connections
//!   start with the right lifecycle. A remembered revision the server no longer speaks falls
//!   back to the probe.
//! - **Credentials.** The stored bearer token and custom headers go on every request as headers
//!   (`ToolHeaders::entries`), marked sensitive.
//! - **Listing.** Every page of `tools/list`, with the smallest `ttlMs` as the cache hint, at
//!   most 16 pages and `MCP_RESPONSE_MAX_BYTES` of definitions in all.
//! - **Calls.** One connection per server per run, opened on its first call. Each call has its
//!   own timeout (`TOOL_TIMEOUT` past it). When a server answers that a tool is unknown or its
//!   headers do not match, the tools are listed again on the connection; a tool still listed is
//!   called once more after a header mismatch, and one that is gone fails with [`TOOL_UNKNOWN`].
//! - **Caps.** Every JSON response and SSE event is capped at `MCP_RESPONSE_MAX_BYTES` (rmcp
//!   rejects a larger one instead of buffering it); core cuts result text to
//!   `MCP_RESULT_MAX_BYTES`.
//!
//! Rules:
//!
//! - Hand rmcp the client that `copilot_core::Outbound::pick` returns for the endpoint's URL and
//!   network scope, from the `Outbound<reqwest::Client>` the composition root injects (built by
//!   `copilot-security` for MCP). Never build a client here, and never enable rmcp's `reqwest`
//!   feature, which turns on reqwest's aws-lc-rs provider.
//! - Log a request the outbound policy refused as `mcp.http.blocked`, other failures as
//!   `mcp.http.failed`. Never echo a response body, an address, or a credential, and never log
//!   tool arguments or results.
//! - No rmcp types leave this crate, so an rmcp major bump touches only this crate.
//! - No `axum`, `sqlx`, `utoipa`, contracts, database, middleware, other adapter, or rustauth
//!   crates (`make boundaries`).

mod connection;
mod convert;
mod errors;
mod host;
mod session;

pub use crate::errors::{
    TOOL_SERVER_AUTH, TOOL_SERVER_BLOCKED, TOOL_SERVER_FAILED, TOOL_SERVER_HEADERS,
    TOOL_SERVER_HTTP, TOOL_SERVER_PROTOCOL, TOOL_SERVER_REFUSED, TOOL_SERVER_TIMEOUT,
    TOOL_SERVER_TOO_LARGE, TOOL_SERVER_UNREACHABLE, TOOL_UNKNOWN,
};
pub use crate::host::{Limits, McpHost};
