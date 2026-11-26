//! The role the server connects as (`copilot_server`). `migrate` connects as the database owner,
//! applies the migrations, then creates or updates this role: it may read and write the rows of
//! every table in `public` and lib-rustauth's `auth` schema, and nothing more. It is not a
//! superuser, cannot create databases, roles, or tables, and cannot change either migration
//! history, so a compromised server process cannot run DDL or `COPY ... TO PROGRAM`.

use sqlx::AssertSqlSafe;

use crate::operations::migrate::AUTH_SCHEMA;
use crate::operations::readonly::GrantError;
use crate::operations::readonly::grant::quote;
use crate::types::DbPool;

/// The server's role. `DB_USER` of the server names it; `DB_SERVER_PASSWORD` of `migrate` sets
/// its password.
pub const SERVER_ROLE: &str = "copilot_server";

/// The tables that record applied migrations, which the server reads (lib-rustauth checks the
/// `auth` schema at startup) but never writes.
const HISTORY: [&str; 2] = ["public._sqlx_migrations", "auth._sqlx_migrations"];

/// Statements that create or update the role and its grants on the tables that exist now. Run
/// them after every migration, so tables a migration adds are granted too.
pub fn statements(database: &str, password: &str) -> Result<Vec<String>, GrantError> {
    let escaped_password = password.replace('\'', "''");
    let quoted_database = quote(database)?;
    let role = SERVER_ROLE;
    let mut statements = vec![
        format!(
            "DO $$\nBEGIN\n  IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = '{role}') THEN\n    CREATE USER {role};\n  END IF;\nEND\n$$;"
        ),
        format!(
            "ALTER USER {role} WITH NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION \
             NOBYPASSRLS PASSWORD '{escaped_password}';"
        ),
        format!("GRANT CONNECT ON DATABASE {quoted_database} TO {role};"),
    ];
    for schema in ["public", AUTH_SCHEMA] {
        statements.extend([
            format!("GRANT USAGE ON SCHEMA {schema} TO {role};"),
            format!("REVOKE CREATE ON SCHEMA {schema} FROM {role};"),
            format!(
                "GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA {schema} TO {role};"
            ),
            format!("GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA {schema} TO {role};"),
        ]);
    }
    statements.extend(HISTORY.map(|table| {
        format!("REVOKE INSERT, UPDATE, DELETE, TRUNCATE ON TABLE {table} FROM {role};")
    }));
    Ok(statements)
}

/// Creates or updates the role with `password` and grants it the rows of every table.
pub async fn grant(pool: &DbPool, database: &str, password: &str) -> Result<(), GrantError> {
    for statement in statements(database, password)? {
        sqlx::raw_sql(AssertSqlSafe(statement))
            .execute(pool)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sql(password: &str) -> String {
        statements("copilot", password).expect("valid").join("\n")
    }

    #[test]
    fn the_role_has_no_attribute_beyond_login() {
        let sql = sql("secret");
        assert!(sql.contains("CREATE USER copilot_server;"));
        assert!(sql.contains(
            "ALTER USER copilot_server WITH NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION \
             NOBYPASSRLS PASSWORD 'secret';"
        ));
    }

    #[test]
    fn rows_only_in_both_schemas_and_never_the_migration_history() {
        let sql = sql("secret");
        for schema in ["public", "auth"] {
            assert!(sql.contains(&format!(
                "GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA {schema} TO \
                 copilot_server;"
            )));
            assert!(sql.contains(&format!(
                "REVOKE CREATE ON SCHEMA {schema} FROM copilot_server;"
            )));
        }
        assert!(!sql.contains("TRUNCATE ON ALL"));
        assert!(sql.contains(
            "REVOKE INSERT, UPDATE, DELETE, TRUNCATE ON TABLE auth._sqlx_migrations FROM \
             copilot_server;"
        ));
    }

    #[test]
    fn escapes_quotes_and_refuses_odd_database_names() {
        assert!(sql("pa'ss").contains("PASSWORD 'pa''ss'"));
        assert!(statements("copilot; DROP", "secret").is_err());
    }
}
