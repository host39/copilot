//! Maps [`DbConfig`] into `sqlx` connection and pool options.

use std::time::Duration;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};

use crate::types::{DbConfig, DbSsl};

const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

pub fn connect_options(config: &DbConfig) -> PgConnectOptions {
    let options = PgConnectOptions::new()
        .host(&config.host)
        .port(config.port)
        .database(&config.database)
        .username(&config.username)
        .password(&config.password);

    match &config.ssl {
        DbSsl::Disabled => options.ssl_mode(PgSslMode::Disable),
        DbSsl::Enabled { root_cert: None } => options.ssl_mode(PgSslMode::VerifyFull),
        DbSsl::Enabled {
            root_cert: Some(path),
        } => options.ssl_mode(PgSslMode::VerifyFull).ssl_root_cert(path),
    }
}

/// Pool sizing from the config. Callers may override, for example `max_connections(1)` for
/// one-shot migrate and seed runs.
pub fn pool_options(config: &DbConfig) -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(config.pool_max)
        .acquire_timeout(config.connection_timeout)
        .idle_timeout(IDLE_TIMEOUT)
}
