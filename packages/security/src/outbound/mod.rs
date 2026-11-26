//! Outbound HTTP to providers and MCP servers: the hardened clients and the address checks they
//! apply (`docs/design/copilot.md`, Outbound Requests). The routing rule ([`copilot_core::Outbound`]),
//! the refusal type ([`copilot_core::Blocked`]), and the response caps live in core, because the
//! adapters that use them may not depend on this crate.

pub mod client;
pub(crate) mod peer;
pub(crate) mod policy;
pub(crate) mod resolver;

pub use crate::outbound::client::{Purpose, clients};
