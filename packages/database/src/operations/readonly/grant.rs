//! Creates the `readonly` role and limits it to `SELECT` on registered tables.

use sqlx::AssertSqlSafe;

use crate::operations::describe::SCHEMA_TABLES;
use crate::operations::migrate::AUTH_SCHEMA;
use crate::operations::readonly::constants::{READONLY_ROLE, READONLY_STATEMENT_TIMEOUT};
use crate::types::DbPool;

#[derive(Debug, thiserror::Error)]
pub enum GrantError {
    #[error("database.readonly.invalid-identifier")]
    InvalidIdentifier,
    #[error("database.readonly.unknown-omitted-column")]
    UnknownOmittedColumn,
    #[error("database.readonly.no-selectable-columns")]
    NoSelectableColumns,
    #[error("database.readonly.query.failed")]
    Query(#[from] sqlx::Error),
}

/// `GRANT SELECT` for one table, limited to the columns that are not omitted.
pub fn select_grant(
    table: &str,
    all_columns: &[String],
    omitted_columns: &[&str],
) -> Result<String, GrantError> {
    let quoted_table = quote(table)?;
    if omitted_columns.is_empty() {
        return Ok(format!(
            "GRANT SELECT ON TABLE {quoted_table} TO {READONLY_ROLE};"
        ));
    }

    if omitted_columns
        .iter()
        .any(|omitted| !all_columns.iter().any(|column| column == omitted))
    {
        return Err(GrantError::UnknownOmittedColumn);
    }

    let selectable = all_columns
        .iter()
        .filter(|column| !omitted_columns.contains(&column.as_str()))
        .map(|column| quote(column))
        .collect::<Result<Vec<_>, _>>()?;
    if selectable.is_empty() {
        return Err(GrantError::NoSelectableColumns);
    }

    Ok(format!(
        "GRANT SELECT ({}) ON {quoted_table} TO {READONLY_ROLE};",
        selectable.join(", ")
    ))
}

/// Statements that create or update the role. `table_grants` sit between the blanket revoke and
/// the default privileges.
pub fn statements(
    database: &str,
    password: &str,
    table_grants: Vec<String>,
) -> Result<Vec<String>, GrantError> {
    let escaped_password = password.replace('\'', "''");
    let quoted_database = quote(database)?;
    let role = READONLY_ROLE;

    let mut statements = vec![
        format!(
            "DO $$\nBEGIN\n  IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = '{role}') THEN\n    CREATE USER {role}\n      NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;\n  END IF;\nEND\n$$;"
        ),
        format!(
            "DO $$\nBEGIN\n  IF EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'rds_iam') THEN\n    REVOKE rds_iam FROM {role};\n  END IF;\nEND\n$$;"
        ),
        format!("ALTER USER {role} WITH PASSWORD '{escaped_password}';"),
        format!("GRANT CONNECT ON DATABASE {quoted_database} TO {role};"),
        format!("GRANT USAGE ON SCHEMA public TO {role};"),
        // Also revokes every column grant on those tables.
        format!("REVOKE SELECT ON ALL TABLES IN SCHEMA public FROM {role};"),
        // lib-rustauth's accounts, password hashes, and sessions: never readable.
        format!("REVOKE ALL ON ALL TABLES IN SCHEMA {AUTH_SCHEMA} FROM {role};"),
        format!("REVOKE ALL ON SCHEMA {AUTH_SCHEMA} FROM {role};"),
    ];
    statements.extend(table_grants);
    statements.extend([
        format!("GRANT SELECT ON ALL SEQUENCES IN SCHEMA public TO {role};"),
        // A new table stays unreadable until it is registered in `SCHEMA_TABLES` (and `migrate`
        // runs again), so a table with private columns is never readable by default.
        format!("ALTER DEFAULT PRIVILEGES IN SCHEMA public REVOKE SELECT ON TABLES FROM {role};"),
        format!("ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON SEQUENCES TO {role};"),
        format!("ALTER ROLE {role} SET default_transaction_read_only = on;"),
        format!("ALTER ROLE {role} SET statement_timeout = '{READONLY_STATEMENT_TIMEOUT}';"),
        "REVOKE ALL ON FUNCTION pg_catalog.pg_sleep(double precision) FROM PUBLIC;".to_owned(),
        "REVOKE ALL ON FUNCTION pg_catalog.pg_terminate_backend(integer, bigint) FROM PUBLIC;"
            .to_owned(),
        format!("REVOKE ALL ON FUNCTION pg_catalog.pg_sleep(double precision) FROM {role};"),
        format!(
            "REVOKE ALL ON FUNCTION pg_catalog.pg_terminate_backend(integer, bigint) FROM {role};"
        ),
    ]);
    Ok(statements)
}

/// Applies every statement for the tables in `SCHEMA_TABLES`.
pub async fn grant(pool: &DbPool, database: &str, password: &str) -> Result<(), GrantError> {
    let mut table_grants = Vec::with_capacity(SCHEMA_TABLES.len());
    for table in SCHEMA_TABLES {
        let omitted: Vec<&str> = table
            .omitted_columns
            .iter()
            .map(|column| column.name)
            .collect();
        let columns = if omitted.is_empty() {
            Vec::new()
        } else {
            columns(pool, table.name).await?
        };
        table_grants.push(select_grant(table.name, &columns, &omitted)?);
    }

    for statement in statements(database, password, table_grants)? {
        sqlx::raw_sql(AssertSqlSafe(statement))
            .execute(pool)
            .await?;
    }
    Ok(())
}

async fn columns(pool: &DbPool, table: &str) -> Result<Vec<String>, GrantError> {
    let names = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = $1 ORDER BY ordinal_position",
    )
    .bind(table)
    .fetch_all(pool)
    .await?;
    Ok(names)
}

fn quote(identifier: &str) -> Result<String, GrantError> {
    let mut chars = identifier.chars();
    let valid_start = chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_');
    if valid_start && chars.all(|rest| rest.is_ascii_alphanumeric() || rest == '_') {
        Ok(format!("\"{identifier}\""))
    } else {
        Err(GrantError::InvalidIdentifier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sql(password: &str) -> String {
        statements("copilot", password, Vec::new())
            .expect("valid")
            .join("\n")
    }

    fn names(columns: &[&str]) -> Vec<String> {
        columns.iter().map(|column| (*column).to_owned()).collect()
    }

    #[test]
    fn creates_readonly_user_with_password_auth() {
        let sql = sql("secret-pass");
        assert!(sql.contains("CREATE USER readonly"));
        assert!(sql.contains("ALTER USER readonly WITH PASSWORD 'secret-pass'"));
        assert!(sql.contains("REVOKE rds_iam FROM readonly"));
        assert!(!sql.contains("GRANT rds_iam"));
    }

    #[test]
    fn escapes_single_quotes_in_passwords() {
        assert!(sql("pa'ss").contains("ALTER USER readonly WITH PASSWORD 'pa''ss'"));
    }

    #[test]
    fn includes_read_only_grants_and_session_limits() {
        let sql = sql("secret-pass");
        assert!(sql.contains("GRANT CONNECT ON DATABASE \"copilot\" TO readonly"));
        assert!(sql.contains("REVOKE SELECT ON ALL TABLES IN SCHEMA public FROM readonly"));
        assert!(sql.contains("REVOKE ALL ON ALL TABLES IN SCHEMA auth FROM readonly"));
        assert!(sql.contains("REVOKE ALL ON SCHEMA auth FROM readonly"));
        assert!(!sql.contains("GRANT SELECT ON TABLES"));
        assert!(
            sql.contains("IN SCHEMA public REVOKE SELECT ON TABLES FROM readonly"),
            "new tables stay unreadable until registered"
        );
        assert!(sql.contains("default_transaction_read_only = on"));
        assert!(sql.contains("statement_timeout = '30s'"));
        assert!(
            sql.contains(
                "REVOKE ALL ON FUNCTION pg_catalog.pg_sleep(double precision) FROM PUBLIC"
            )
        );
    }

    #[test]
    fn grants_only_non_omitted_columns() {
        let statement = select_grant("users", &names(&["id", "token"]), &["token"]).expect("valid");
        assert_eq!(statement, "GRANT SELECT (\"id\") ON \"users\" TO readonly;");
    }

    #[test]
    fn grants_full_table_when_nothing_is_omitted() {
        let statement = select_grant("teams", &names(&["id"]), &[]).expect("valid");
        assert_eq!(statement, "GRANT SELECT ON TABLE \"teams\" TO readonly;");
    }

    #[test]
    fn rejects_unknown_omitted_columns() {
        let error = select_grant("users", &names(&["id"]), &["token"]).expect_err("invalid");
        assert_eq!(
            error.to_string(),
            "database.readonly.unknown-omitted-column"
        );
    }

    #[test]
    fn rejects_omitting_every_column() {
        let error = select_grant("users", &names(&["token"]), &["token"]).expect_err("invalid");
        assert_eq!(error.to_string(), "database.readonly.no-selectable-columns");
    }

    #[test]
    fn rejects_invalid_identifiers() {
        let error = select_grant("bad-name", &names(&["id"]), &["id"]).expect_err("invalid");
        assert_eq!(error.to_string(), "database.readonly.invalid-identifier");
    }
}
