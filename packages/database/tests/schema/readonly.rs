//! What the readonly role reads after `migrate` grants it (docs/design/copilot.md, "Readonly
//! Role"): registered tables without their sealed columns, and nothing else.

use copilot_database::DbPool;
use copilot_database::operations::describe::SCHEMA_TABLES;
use copilot_database::operations::migrate::AUTH_SCHEMA;
use copilot_database::operations::readonly::{self, READONLY_ROLE};

use crate::fixtures::{self, Graph, INSUFFICIENT_PRIVILEGE, TABLES};

/// The only tables the readonly role may read.
const READABLE: &[&str] = &["files", "models", "providers", "schedules", "tool_servers"];

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn reads_registered_columns_only_and_nothing_in_auth(pool: DbPool) {
    fixtures::migrated(&pool).await;
    let user = fixtures::user(&pool).await;
    Graph::insert(&pool, user).await;
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("database named");
    // The role is cluster-wide: keep the password the developer's environment gives it (this is
    // the only test that grants, so tests never race on the role).
    let password =
        std::env::var("DB_READONLY_PASSWORD").unwrap_or_else(|_| READONLY_ROLE.to_owned());
    readonly::grant(&pool, &database, &password)
        .await
        .expect("readonly role granted");

    let mut registered: Vec<&str> = SCHEMA_TABLES.iter().map(|table| table.name).collect();
    registered.sort_unstable();
    assert_eq!(registered, READABLE);

    // A table created after the grant stays unreadable until it is registered.
    sqlx::raw_sql("CREATE TABLE later (id uuid PRIMARY KEY, notes text)")
        .execute(&pool)
        .await
        .expect("table created");
    let usage: bool = sqlx::query_scalar("SELECT has_schema_privilege($1, $2, 'USAGE')")
        .bind(READONLY_ROLE)
        .bind(AUTH_SCHEMA)
        .fetch_one(&pool)
        .await
        .expect("privilege read");
    assert!(!usage, "no usage on the auth schema");
    let columns: Vec<(String, String)> = sqlx::query_as(
        "SELECT table_name::text, column_name::text FROM information_schema.columns \
         WHERE table_schema = 'public' ORDER BY table_name, ordinal_position",
    )
    .fetch_all(&pool)
    .await
    .expect("columns listed");
    let auth_tables = fixtures::tables(&pool, AUTH_SCHEMA).await;
    assert!(auth_tables.iter().any(|table| table == "users"));

    let mut connection = pool.acquire().await.expect("connection");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("SET ROLE {READONLY_ROLE}")))
        .execute(&mut *connection)
        .await
        .expect("role set");

    for (table, column) in &columns {
        let readable = SCHEMA_TABLES.iter().any(|registered| {
            registered.name == table
                && !registered
                    .omitted_columns
                    .iter()
                    .any(|omitted| omitted.name == column)
        });
        let sql = format!("SELECT {column} FROM {table} LIMIT 1");
        let expected = (!readable).then_some(INSUFFICIENT_PRIVILEGE);
        assert_eq!(
            fixtures::failure(&mut *connection, &sql).await.as_deref(),
            expected,
            "{sql}"
        );
        assert!(
            !(readable && column.ends_with("_sealed")),
            "{table}.{column} is sealed"
        );
    }

    let mut denied: Vec<String> = TABLES
        .iter()
        .filter(|table| !READABLE.contains(table))
        .map(|table| format!("SELECT * FROM {table}"))
        .collect();
    denied.extend([
        "SELECT * FROM later".to_owned(),
        "SELECT api_key_sealed FROM providers".to_owned(),
        "SELECT headers_sealed FROM tool_servers".to_owned(),
        "SELECT * FROM providers".to_owned(),
        "SELECT * FROM tool_servers".to_owned(),
    ]);
    denied.extend(
        auth_tables
            .iter()
            .map(|table| format!("SELECT * FROM {AUTH_SCHEMA}.{table}")),
    );
    for sql in &denied {
        assert_eq!(
            fixtures::failure(&mut *connection, sql).await.as_deref(),
            Some(INSUFFICIENT_PRIVILEGE),
            "{sql}"
        );
    }
    for table in ["models", "schedules", "files"] {
        let sql = format!("SELECT * FROM {table}");
        assert_eq!(
            fixtures::failure(&mut *connection, &sql).await,
            None,
            "{sql}"
        );
    }

    sqlx::raw_sql("RESET ROLE")
        .execute(&mut *connection)
        .await
        .expect("role reset");
}
