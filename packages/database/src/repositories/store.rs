//! The database adapter's store, the placeholder its unfinished methods return, and the helpers
//! every repository module shares (error keys, `jsonb` records, integer columns).

use std::fmt;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use copilot_core::{Clock, DomainError, DomainResult};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::types::Json;

use crate::records::RecordError;
use crate::types::DbPool;

/// Every repository port of `copilot-core` on one Postgres pool. Cheap to clone (the pool is a
/// handle). Timestamps a repository sets itself (`created_at`, `updated_at` of rows no entity
/// carries) come from `clock`, the same `Clock` the services use.
#[derive(Clone)]
pub struct PgStore {
    pool: DbPool,
    clock: Arc<dyn Clock>,
}

impl PgStore {
    pub fn new(pool: DbPool, clock: Arc<dyn Clock>) -> Self {
        Self { pool, clock }
    }

    pub fn pool(&self) -> &DbPool {
        &self.pool
    }

    /// The current time from the injected `Clock`.
    pub(crate) fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }
}

impl fmt::Debug for PgStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PgStore")
            .field("pool", &self.pool)
            .finish_non_exhaustive()
    }
}

/// Maps a driver error to an internal error with the dotted key of the failed operation
/// (`database.<table>.<operation>.failed`).
pub(crate) fn failed(key: &'static str) -> impl FnOnce(sqlx::Error) -> DomainError {
    move |error| DomainError::internal(key, error)
}

/// Maps a stored record this version cannot read to an internal error naming the column
/// (`database.<table>.<column>.invalid`).
pub(crate) fn invalid(key: &'static str) -> impl FnOnce(RecordError) -> DomainError {
    move |error| DomainError::internal(key, error)
}

/// A `jsonb` value for a record.
pub(crate) fn json<T: Serialize>(record: T) -> Json<T> {
    Json(record)
}

/// Reads a `jsonb` column into a record, or the internal error `key`.
pub(crate) fn record<T: DeserializeOwned>(
    value: serde_json::Value,
    key: &'static str,
) -> DomainResult<T> {
    serde_json::from_value(value).map_err(|error| DomainError::internal(key, error))
}

/// A stored text enum (`text_enum` spelling), or the internal error `key`.
pub(crate) fn text<T: std::str::FromStr>(value: &str, key: &'static str) -> DomainResult<T> {
    value
        .parse()
        .map_err(|_| DomainError::internal(key, RecordError::Value(value.to_owned())))
}

/// An `integer` column value. Core keeps token limits at most `i32::MAX`.
pub(crate) fn int(value: Option<u32>) -> Option<i32> {
    value.map(|value| i32::try_from(value).unwrap_or(i32::MAX))
}

/// A stored `integer` that core reads as unsigned (`CHECK (... >= 0)` holds it).
pub(crate) fn uint(value: Option<i32>) -> Option<u32> {
    value.and_then(|value| u32::try_from(value).ok())
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    #[test]
    fn unreadable_values_name_the_column_and_the_value() {
        let error = text::<copilot_core::Role>("robot", "database.messages.role.invalid")
            .expect_err("unknown role");
        assert_eq!(error.to_string(), "database.messages.role.invalid");
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("database.records.value.invalid robot")
        );
    }

    #[test]
    fn integer_columns_stay_in_range() {
        assert_eq!(int(Some(u32::MAX)), Some(i32::MAX));
        assert_eq!(int(Some(8_000)), Some(8_000));
        assert_eq!(uint(Some(-1)), None);
        assert_eq!(uint(Some(128_000)), Some(128_000));
    }
}
