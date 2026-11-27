//! Ports used by several domains: traits and types that adapters in other crates implement.
//! Repository ports live with their domain (`domains/<name>/ports.rs`). There are no identity
//! ports beyond `Accounts`, which resolves the owners of scheduled runs: lib-rustauth owns users,
//! passwords, and sessions.

pub mod accounts;
pub mod clock;
pub mod llm_gateway;
pub mod logger;
pub mod secret_cipher;
pub mod tool_host;
pub mod turn_sink;
pub mod worker;
