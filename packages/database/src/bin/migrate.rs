//! Applies migrations, then creates or updates the `readonly` role and its grants.
//!
//! `make migrate` runs this with `apps/api/.env.local` loaded.

use std::process::ExitCode;

use copilot_core::BoxError;
use copilot_database::config::{connect_options, pool_options};
use copilot_database::operations::{migrate, readonly, start};
use copilot_database::{DbConfig, Env, ReadonlyConfig};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let env = Env::load();
    if let Err(error) = start(&env) {
        #[allow(clippy::print_stderr)]
        {
            eprintln!("database.migrate.start.failed {error}");
        }
        return ExitCode::FAILURE;
    }

    match run(&env).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, source = ?error.source(), "database.migrate.failed");
            ExitCode::FAILURE
        }
    }
}

async fn run(env: &Env) -> Result<(), BoxError> {
    let config = DbConfig::parse(env)?;
    let readonly_config = ReadonlyConfig::parse(env)?;
    let pool = pool_options(&config)
        .max_connections(1)
        .connect_with(connect_options(&config))
        .await?;

    let result = async {
        let migrations = migrate::run(&pool).await?;
        tracing::info!(migrations, "database.migrate.applied");

        match readonly_config {
            Some(readonly_config) => {
                readonly::grant(&pool, &config.database, &readonly_config.password).await?;
                tracing::info!(role = readonly::READONLY_ROLE, "database.readonly.granted");
            }
            None => tracing::info!("database.readonly.skipped"),
        }
        Ok::<(), BoxError>(())
    }
    .await;

    pool.close().await;
    result
}
