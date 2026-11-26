//! What the server's role (`copilot_server`) may do after `migrate` grants it: read and write the
//! rows of every table in `public` and `auth`, read but never write either migration history,
//! and nothing a superuser or the database owner can.

use copilot_database::DbPool;
use copilot_database::operations::migrate::AUTH_SCHEMA;
use copilot_database::operations::server::{self, SERVER_ROLE};

use crate::fixtures::{self, Graph, INSUFFICIENT_PRIVILEGE};

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn the_server_role_writes_rows_and_nothing_else(pool: DbPool) {
    fixtures::migrated(&pool).await;
    let user = fixtures::user(&pool).await;
    Graph::insert(&pool, user).await;
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("database named");
    // The role is cluster-wide; this is the only test that grants it.
    let password = std::env::var("DB_SERVER_PASSWORD").unwrap_or_else(|_| SERVER_ROLE.to_owned());
    server::grant(&pool, &database, &password)
        .await
        .expect("server role granted");
    // Granting again (every `migrate` run) changes nothing.
    server::grant(&pool, &database, &password)
        .await
        .expect("server role granted again");

    let attributes: (bool, bool, bool, bool, bool) = sqlx::query_as(
        "SELECT rolsuper, rolcreatedb, rolcreaterole, rolreplication, rolbypassrls \
         FROM pg_roles WHERE rolname = $1",
    )
    .bind(SERVER_ROLE)
    .fetch_one(&pool)
    .await
    .expect("role read");
    assert_eq!(attributes, (false, false, false, false, false));

    let mut tables = Vec::new();
    for schema in ["public", AUTH_SCHEMA] {
        for table in fixtures::tables(&pool, schema).await {
            tables.push(format!("{schema}.{table}"));
        }
    }
    assert!(tables.iter().any(|table| table == "auth.users"));
    for table in &tables {
        let history = table.ends_with("._sqlx_migrations");
        for privilege in ["SELECT", "INSERT", "UPDATE", "DELETE", "TRUNCATE"] {
            let granted: bool = sqlx::query_scalar("SELECT has_table_privilege($1, $2, $3)")
                .bind(SERVER_ROLE)
                .bind(table)
                .bind(privilege)
                .fetch_one(&pool)
                .await
                .expect("privilege read");
            let expected = match privilege {
                "SELECT" => true,
                "TRUNCATE" => false,
                _ => !history,
            };
            assert_eq!(granted, expected, "{privilege} on {table}");
        }
    }

    let mut connection = pool.acquire().await.expect("connection");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("SET ROLE {SERVER_ROLE}")))
        .execute(&mut *connection)
        .await
        .expect("role set");
    for sql in [
        "UPDATE auth.users SET display_name = 'Renamed'",
        "UPDATE chats SET title = 'Renamed'",
        "DELETE FROM file_contents",
        "SELECT version FROM auth._sqlx_migrations",
    ] {
        assert_eq!(
            fixtures::failure(&mut *connection, sql).await,
            None,
            "{sql}"
        );
    }
    for sql in [
        "CREATE TABLE later (id uuid)",
        "CREATE TABLE auth.later (id uuid)",
        "DROP TABLE chats",
        "ALTER TABLE chats ADD COLUMN extra text",
        "DELETE FROM _sqlx_migrations",
        "DELETE FROM auth._sqlx_migrations",
        "TRUNCATE chats",
        "CREATE ROLE intruder",
        "COPY (SELECT 1) TO PROGRAM 'true'",
    ] {
        assert_eq!(
            fixtures::failure(&mut *connection, sql).await.as_deref(),
            Some(INSUFFICIENT_PRIVILEGE),
            "{sql}"
        );
    }
    sqlx::raw_sql("RESET ROLE")
        .execute(&mut *connection)
        .await
        .expect("role reset");
}
