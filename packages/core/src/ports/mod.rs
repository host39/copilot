//! Ports used by several domains: traits and types that adapters in other crates implement.
//! Repository ports live with their domain (`domains/<name>/ports.rs`). There are no identity
//! ports: lib-rustauth owns users, passwords, and sessions.

pub mod clock;
pub mod llm_gateway;
pub mod logger;
pub mod secret_cipher;
