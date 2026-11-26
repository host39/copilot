//! Security adapters: AES-256-GCM sealing of stored secrets (implements the `SecretCipher` port of
//! `copilot-core`) and the hardened outbound `reqwest::Client` builder, whose DNS resolver applies
//! the core address policy.
//!
//! Rules (see "Secrets" and "Outbound Requests" in `docs/design/copilot.md`):
//!
//! - The composition root builds the outbound clients once and injects them into the provider and
//!   MCP adapters; no other crate builds a `reqwest::Client`.
//! - reqwest uses `rustls-no-provider`: the ring provider is installed once, before the first
//!   client is built ([`crypto::install`]), so aws-lc-rs stays out of the tree.
//! - Passwords and sessions are not here: they come from lib-rustauth.
//! - No `axum`, `sqlx`, `utoipa`, contracts, database, middleware, other adapter, or rustauth
//!   crates (`make boundaries`).

pub mod cipher;
pub mod crypto;
pub mod outbound;

pub use crate::cipher::{AesCipher, FORMAT_VERSION, KEY_LEN, KeyError, SecretKey};
pub use crate::outbound::{Purpose, clients};
