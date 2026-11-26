//! Schema migrations, embedded at compile time from `packages/database/migrations/`.
//!
//! A migration is immutable once it is part of a release or applied to a deployed database: `sqlx`
//! stores a checksum per applied migration and refuses to run when an applied file changes. See
//! `docs/adr/002-database-migrations-and-seed.md`.
//!
//! [`run`] is the one entry point for the `migrate` binary and for database tests. Tests use
//! `#[sqlx::test(migrations = false)]` and call [`run`] first, so they get the same schema as a
//! real install: lib-rustauth's `auth` migrations, then [`MIGRATOR`].

pub mod seed;

use sqlx::migrate::{MigrateError, Migrator};

use crate::types::DbPool;

/// Copilot's own migrations, tracked in `public._sqlx_migrations`. They reference `auth.users`,
/// so they only run after lib-rustauth's (see [`run`]).
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// The Postgres schema lib-rustauth owns. Copilot never writes to it, and the readonly role gets
/// no grant on it.
pub const AUTH_SCHEMA: &str = "auth";

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// lib-rustauth's migrations failed; the source says why.
    #[error("database.migrate.auth.failed")]
    Auth(#[source] rustauth::AuthError),
    #[error("database.migrate.copilot.failed")]
    Copilot(#[from] MigrateError),
}

/// Applies lib-rustauth's pending `auth` migrations (history in `auth._sqlx_migrations`), then
/// Copilot's ([`MIGRATOR`], history in `_sqlx_migrations`), and returns how many Copilot
/// migrations the binary embeds. Both steps skip applied migrations, so running it again is a
/// no-op.
pub async fn run(pool: &DbPool) -> Result<usize, MigrationError> {
    rustauth::migrate(pool)
        .await
        .map_err(MigrationError::Auth)?;
    MIGRATOR.run(pool).await?;
    Ok(MIGRATOR.iter().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run `make test-db`"]
    async fn applies_every_migration_once(pool: DbPool) {
        let embedded = run(&pool).await.expect("first run applies the migrations");
        let again = run(&pool).await.expect("second run is a no-op");
        assert_eq!(again, embedded);

        let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .expect("the history table exists");
        assert_eq!(usize::try_from(applied).expect("non-negative"), embedded);

        let auth: i64 = sqlx::query_scalar("SELECT count(*) FROM auth._sqlx_migrations")
            .fetch_one(&pool)
            .await
            .expect("rustauth keeps its own history in the auth schema");
        assert!(auth > 0, "rustauth's migrations run first");
    }
}
