//! The database adapter's store and the placeholder its unfinished methods return.

use copilot_core::{DomainError, DomainResult};

use crate::types::DbPool;

/// Every repository port of `copilot-core` on one Postgres pool. Cheap to clone (the pool is a
/// handle).
#[derive(Clone, Debug)]
pub struct PgStore {
    pool: DbPool,
}

impl PgStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &DbPool {
        &self.pool
    }
}

/// A repository method that roadmap step P2.A has not written yet.
#[derive(Debug, thiserror::Error)]
#[error("{0} is not implemented yet")]
pub struct Unimplemented(pub &'static str);

/// The result of an unfinished method: an internal error (`500`, never reached through a route
/// that answers `501`) that names the method in the log.
pub(crate) fn pending<T>(method: &'static str) -> DomainResult<T> {
    Err(DomainError::internal(
        "database.repository.unimplemented",
        Unimplemented(method),
    ))
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    #[test]
    fn unfinished_methods_fail_with_an_internal_error_naming_the_method() {
        let error = pending::<()>("SettingsRepository::get").expect_err("unfinished");
        assert_eq!(error.to_string(), "database.repository.unimplemented");
        assert_eq!(error.code(), "INTERNAL_ERROR");
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("SettingsRepository::get is not implemented yet")
        );
    }
}
