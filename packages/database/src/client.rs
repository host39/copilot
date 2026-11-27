use std::time::Duration;

use crate::config::{connect_options, pool_options};
use crate::types::{DbConfig, DbPool};

/// Readiness checks give up after this long so a down database cannot stall the probe.
const PING_TIMEOUT: Duration = Duration::from_secs(3);

/// Creates the connection pool. Connections open on first use, but the API still needs a
/// reachable, migrated database to start (lib-rustauth checks its schema, and startup recovery
/// runs a query); `/health/ready` reports outages that happen after startup.
pub fn create_pool(config: &DbConfig) -> DbPool {
    pool_options(config).connect_lazy_with(connect_options(config))
}

/// Closes every connection and waits for in-flight queries.
pub async fn close(pool: &DbPool) {
    pool.close().await;
}

/// `true` when the database answers `SELECT 1` in time.
pub async fn ping(pool: &DbPool) -> bool {
    let query = sqlx::query("SELECT 1").execute(pool);
    matches!(tokio::time::timeout(PING_TIMEOUT, query).await, Ok(Ok(_)))
}
