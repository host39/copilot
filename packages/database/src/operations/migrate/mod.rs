//! Schema migrations, embedded at compile time from `packages/database/migrations/`.
//!
//! Files are append-only once committed: `sqlx` stores a checksum per applied migration and
//! refuses to run when a committed file changes. See `docs/adr/002-database-migrations-and-seed.md`.

pub mod seed;

use sqlx::migrate::{MigrateError, Migrator};

use crate::types::DbPool;

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Applies pending migrations and returns how many the binary embeds in total.
pub async fn run(pool: &DbPool) -> Result<usize, MigrateError> {
    MIGRATOR.run(pool).await?;
    Ok(MIGRATOR.iter().count())
}
