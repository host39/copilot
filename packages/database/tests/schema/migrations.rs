//! The migrations as a whole: up, down to zero, up again, and column alignment.

use copilot_database::DbPool;
use copilot_database::operations::migrate::{self, AUTH_SCHEMA, MIGRATOR};

use crate::fixtures::{self, TABLES};

/// Columns added with `ALTER TABLE ... ADD COLUMN`, which always appends at the end and so may
/// break the alignment order (`postgres-column-alignment`). Add `(table, column)` here when a
/// migration does that.
const APPENDED: &[(&str, &str)] = &[];

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn reverts_to_zero_and_back_without_touching_the_auth_schema(pool: DbPool) {
    fixtures::migrated(&pool).await;
    let mut expected: Vec<&str> = TABLES.to_vec();
    expected.push("_sqlx_migrations");
    expected.sort_unstable();
    assert_eq!(fixtures::tables(&pool, "public").await, expected);

    let user = fixtures::user(&pool).await;
    let auth_tables = fixtures::tables(&pool, AUTH_SCHEMA).await;
    let auth_history = history(&pool, "auth._sqlx_migrations").await;
    assert!(!auth_history.is_empty());

    MIGRATOR
        .undo(&pool, 0)
        .await
        .expect("every migration reverts");
    assert_eq!(
        fixtures::tables(&pool, "public").await,
        ["_sqlx_migrations"]
    );
    assert!(history(&pool, "_sqlx_migrations").await.is_empty());
    assert_eq!(fixtures::tables(&pool, AUTH_SCHEMA).await, auth_tables);
    assert_eq!(history(&pool, "auth._sqlx_migrations").await, auth_history);
    assert_eq!(
        fixtures::count(&pool, "auth.users", "id = $1", user).await,
        1
    );

    let embedded = migrate::run(&pool).await.expect("migrations apply again");
    assert_eq!(fixtures::tables(&pool, "public").await, expected);
    assert_eq!(history(&pool, "_sqlx_migrations").await.len(), embedded);
    assert_eq!(history(&pool, "auth._sqlx_migrations").await, auth_history);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn columns_follow_the_alignment_order(pool: DbPool) {
    fixtures::migrated(&pool).await;
    let columns: Vec<(String, String, i32)> = sqlx::query_as(
        "SELECT c.relname::text, a.attname::text, t.typlen::int4 \
         FROM pg_attribute a \
         JOIN pg_class c ON c.oid = a.attrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         JOIN pg_type t ON t.oid = a.atttypid \
         WHERE n.nspname = 'public' AND c.relkind = 'r' AND a.attnum > 0 \
           AND NOT a.attisdropped AND c.relname <> '_sqlx_migrations' \
         ORDER BY c.relname, a.attnum",
    )
    .fetch_all(&pool)
    .await
    .expect("columns listed");
    assert!(!columns.is_empty());

    let mut previous: Option<(&str, u8)> = None;
    for (table, column, length) in &columns {
        if APPENDED.contains(&(table.as_str(), column.as_str())) {
            continue;
        }
        let rank = rank(*length);
        if let Some((previous_table, previous_rank)) = previous
            && previous_table == table
        {
            assert!(
                rank >= previous_rank,
                "{table}.{column} breaks the column alignment order"
            );
        }
        previous = Some((table, rank));
    }
}

/// The alignment group of a type by its storage length: 8-byte, 4-byte, 2-byte, `uuid`,
/// `boolean`, then variable length (`text`, `bytea`, `jsonb`, arrays).
fn rank(length: i32) -> u8 {
    match length {
        8 => 0,
        4 => 1,
        2 => 2,
        16 => 3,
        1 => 4,
        _ => 5,
    }
}

/// The applied versions in a migration history table, in order.
async fn history(pool: &DbPool, table: &str) -> Vec<i64> {
    let sql = format!("SELECT version FROM {table} ORDER BY version");
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .fetch_all(pool)
        .await
        .expect("history read")
}
