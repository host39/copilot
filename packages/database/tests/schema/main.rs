//! Tests of the schema as a whole (`make test-db`): migrations up, down, and up again with
//! lib-rustauth's `auth` schema intact, column alignment, tenant isolation keys, `CHECK`
//! constraints, the readonly role, and what deleting rows cascades or clears.
//!
//! Every test gets a throwaway database (`#[sqlx::test(migrations = false)]`) and starts with
//! [`fixtures::migrated`], which runs the same `migrate::run` as a real install.

mod constraints;
mod deletes;
mod fixtures;
mod migrations;
mod readonly;
