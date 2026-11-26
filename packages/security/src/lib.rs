//! Security adapters: AES-256-GCM sealing of stored secrets (implements the `SecretCipher` port of
//! `copilot-core`) and the hardened outbound `reqwest::Client` builder, whose DNS resolver applies
//! the core address policy. Roadmap step P1.6 fills this crate in.
//!
//! Rules (see "Secrets" and "Outbound Requests" in `docs/design/copilot.md`):
//!
//! - The composition root builds the outbound clients once and injects them into the provider and
//!   MCP adapters; no other crate builds a `reqwest::Client`.
//! - reqwest uses `rustls-no-provider`: each binary installs the ring provider at startup, before
//!   the first client is built, so aws-lc-rs stays out of the tree.
//! - Passwords and sessions are not here: they come from lib-rustauth.
//! - No `axum`, `sqlx`, `utoipa`, contracts, database, middleware, other adapter, or rustauth
//!   crates (`make boundaries`).
