//! `fake-llm`: serves the fake model server (`make fake-llm`, port 4010 by default). Settings
//! come from `FAKE_LLM_*` variables (`tools/fake-llm/README.md`).

use std::net::SocketAddr;
use std::process::ExitCode;

use fake_llm::{Config, app};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::main]
async fn main() -> ExitCode {
    let config = match Config::load() {
        Ok(config) => config,
        Err(error) => {
            #[allow(clippy::print_stderr)]
            {
                eprintln!("{error}");
            }
            return ExitCode::FAILURE;
        }
    };
    // rmcp logs every session at `info`; only its warnings are worth seeing here.
    let filter = Targets::new()
        .with_default(LevelFilter::INFO)
        .with_target("rmcp", LevelFilter::WARN);
    let installed = tracing_subscriber::fmt()
        .compact()
        .with_target(false)
        .finish()
        .with(filter)
        .try_init();
    if let Err(error) = installed {
        #[allow(clippy::print_stderr)]
        {
            eprintln!("fake-llm.logger.failed {error}");
        }
        return ExitCode::FAILURE;
    }

    let address = SocketAddr::new(config.host, config.port);
    let listener = match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(%address, error = %error, "fake-llm.server.bind-failed");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(%address, key = %config.key, "fake-llm.server.listening");
    match axum::serve(listener, app(config)).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, "fake-llm.server.failed");
            ExitCode::FAILURE
        }
    }
}
