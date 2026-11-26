//! Long-running HTTP server (the default entrypoint).

use std::process::ExitCode;

use copilot_api::helpers::bootstrap::ApiContext;
use copilot_core::BoxError;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> ExitCode {
    let context = match ApiContext::create() {
        Ok(context) => context,
        Err(error) => {
            // The logger may not be installed yet, so this is the one place that writes to stderr.
            #[allow(clippy::print_stderr)]
            {
                eprintln!("api.server.start.failed {error}");
            }
            return ExitCode::FAILURE;
        }
    };

    match serve(context).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, "api.server.failed");
            ExitCode::FAILURE
        }
    }
}

async fn serve(context: ApiContext) -> Result<(), BoxError> {
    let host = context.config.host.clone();
    let port = context.config.port;
    let listener = TcpListener::bind((host.as_str(), port)).await?;
    tracing::info!(
        host,
        port,
        environment = %context.config.environment,
        "api.server.listening"
    );

    axum::serve(listener, context.app.clone())
        .with_graceful_shutdown(shutdown())
        .await?;

    context.close().await;
    tracing::info!("api.server.stopped");
    Ok(())
}

/// Resolves on Ctrl+C or SIGTERM so in-flight requests can finish.
async fn shutdown() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::warn!(error = %error, "api.server.signal.unavailable");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = interrupt => tracing::info!(signal = "SIGINT", "api.server.shutdown"),
        () = terminate => tracing::info!(signal = "SIGTERM", "api.server.shutdown"),
    }
}
