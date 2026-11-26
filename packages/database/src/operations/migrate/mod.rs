//! Schema migrations, embedded at compile time from `packages/database/migrations/`.
//!
//! A migration is immutable once it is part of a release or applied to a deployed database: `sqlx`
//! stores a checksum per applied migration and refuses to run when an applied file changes. See
//! `docs/adr/002-database-migrations-and-seed.md`.
//!
//! [`run`] is the one entry point for the `migrate` binary and for database tests. Tests use
//! `#[sqlx::test(migrations = false)]` and call [`run`] first, so they get the same schema as a
//! real install (from P1.4 on, lib-rustauth's `auth` migrations followed by [`MIGRATOR`]).

pub mod seed;

use sqlx::migrate::{MigrateError, Migrator};

use crate::types::DbPool;

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Applies pending migrations and returns how many the binary embeds in total.
pub async fn run(pool: &DbPool) -> Result<usize, MigrateError> {
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
    }
}
