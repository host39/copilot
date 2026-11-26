//! Applies the SQL files in one seed folder (`local` by default).
//!
//! `make seed` runs this with `apps/api/.env.local` loaded. Pass a folder name to pick another
//! one, for example `cargo run -p copilot-database --bin seed -- dev`.

use std::path::PathBuf;
use std::process::ExitCode;

use copilot_core::{BoxError, Environment};
use copilot_database::config::{connect_options, pool_options};
use copilot_database::operations::migrate::seed;
use copilot_database::operations::start;
use copilot_database::{DbConfig, Env};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let env = Env::load();
    let environment = match start(&env) {
        Ok(environment) => environment,
        Err(error) => {
            #[allow(clippy::print_stderr)]
            {
                eprintln!("database.seed.start.failed {error}");
            }
            return ExitCode::FAILURE;
        }
    };

    if environment == Environment::Production {
        tracing::error!("database.seed.refused.production");
        return ExitCode::FAILURE;
    }

    let folder = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "local".to_owned());
    match run(&env, &folder).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, source = ?error.source(), "database.seed.failed");
            ExitCode::FAILURE
        }
    }
}

async fn run(env: &Env, folder: &str) -> Result<(), BoxError> {
    let root = env
        .get("SEED_DIR")
        .map_or_else(|| PathBuf::from(seed::SEED_ROOT), PathBuf::from);
    let paths = seed::files(&root.join(folder))?;
    if paths.is_empty() {
        tracing::warn!(folder, "database.seed.skipped.no-files");
        return Ok(());
    }

    let config = DbConfig::parse(env)?;
    let pool = pool_options(&config)
        .max_connections(1)
        .connect_with(connect_options(&config))
        .await?;

    let result = async {
        let skip_query = env
            .get("SEED_SKIP_QUERY")
            .filter(|_| env.get("SEED_SKIP_IF_POPULATED") == Some("1"));
        if let Some(query) = skip_query
            && seed::populated(&pool, query).await?
        {
            tracing::info!(folder, "database.seed.skipped.populated");
            return Ok(());
        }

        let applied = seed::apply(&pool, &paths).await?;
        tracing::info!(folder, files = applied.len(), "database.seed.completed");
        Ok::<(), BoxError>(())
    }
    .await;

    pool.close().await;
    result
}
