//! Persistence: Postgres client, environment parsing, migrations, seed, and the `sqlx`
//! implementations of the repository traits defined in `copilot-core`.
//!
//! Binaries in `src/bin/` apply migrations (`migrate`) and SQL seed files (`seed`); the API only
//! reads and writes data through repositories.

pub mod client;
pub mod config;
pub mod env;
pub mod operations;
pub mod records;
pub mod repositories;
pub mod types;

pub use crate::client::{close, create_pool, ping};
pub use crate::env::{Env, EnvError, EnvParser};
pub use crate::repositories::PgStore;
pub use crate::types::{DbConfig, DbPool, DbSsl, ReadonlyConfig, ServerRoleConfig};
