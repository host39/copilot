use std::path::PathBuf;
use std::time::Duration;

use sqlx::PgPool;

/// Connection pool shared by repositories.
pub type DbPool = PgPool;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DbConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    pub ssl: DbSsl,
    pub pool_max: u32,
    pub connection_timeout: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DbSsl {
    Disabled,
    /// Verify the server certificate and hostname, against `root_cert` when set or the bundled
    /// Mozilla roots otherwise.
    Enabled {
        root_cert: Option<PathBuf>,
    },
}

/// The password `migrate` gives the `copilot_server` role (`DB_SERVER_PASSWORD`).
#[derive(Clone, Eq, PartialEq)]
pub struct ServerRoleConfig {
    pub password: String,
}

impl std::fmt::Debug for ServerRoleConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerRoleConfig")
            .field("password", &"***")
            .finish()
    }
}

/// Credentials for the `readonly` role that `migrate` creates and grants.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadonlyConfig {
    pub username: String,
    pub password: String,
}
