//! Registry of application tables and what the `readonly` role may read from them.
//!
//! Register each table as domains are added so readonly grants stay in sync. `migrate` revokes
//! `SELECT` on every table, then grants it back only for tables listed here.

/// A column the readonly role must not read, with the reason (for example a secret or token).
#[derive(Clone, Copy, Debug)]
pub struct OmittedColumn {
    pub name: &'static str,
    pub reason: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct TableDoc {
    pub name: &'static str,
    pub summary: &'static str,
    pub omitted_columns: &'static [OmittedColumn],
}

pub const SCHEMA_OVERVIEW: &str =
    "Register tables in SCHEMA_TABLES as domains are added so readonly grants stay in sync.";

pub const SCHEMA_TABLES: &[TableDoc] = &[];
