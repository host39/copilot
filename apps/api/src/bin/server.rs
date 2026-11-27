//! Long-running HTTP server (the default entrypoint).

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use copilot_api::helpers::bootstrap::{ApiContext, drain};
use copilot_api::helpers::serve::serve as run;
use copilot_core::BoxError;
use copilot_core::shared::constants::HEADER_READ_TIMEOUT_SECS;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> ExitCode {
    let context = match ApiContext::create().await {
        Ok(context) => context,
        Err(error) => {
            // The logger may not be installed yet, so this is the one place that writes to stderr.
            // The causes follow the key, for example "Auth schema is out of date: run migrations".
            let mut line = format!("api.server.start.failed {error}");
            let mut source = error.source();
            while let Some(cause) = source {
                line.push_str(&format!(": {cause}"));
                source = cause.source();
            }
            #[allow(clippy::print_stderr)]
            {
                eprintln!("{line}");
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

    // On a signal the server stops accepting connections and the runs drain: live runs get a
    // moment to finish (their streams with them), then the root token cancels the rest and ends
    // every open stream, so the graceful shutdown does not wait on them.
    let runs = Arc::clone(&context.runs);
    let signalled = CancellationToken::new();
    let draining = tokio::spawn({
        let signalled = signalled.clone();
        async move {
            signalled.cancelled().await;
            drain(&runs).await;
        }
    });
    // The peer address feeds lib-rustauth's per-IP limits and `AUTH_TRUSTED_PROXIES`.
    let header_timeout = Duration::from_secs(HEADER_READ_TIMEOUT_SECS);
    run(
        listener,
        context.app.clone(),
        header_timeout,
        shutdown(signalled),
    )
    .await;
    context.close().await;
    draining.abort();
    tracing::info!("api.server.stopped");
    Ok(())
}

/// Resolves on Ctrl+C or SIGTERM, after cancelling `signalled`.
async fn shutdown(signalled: CancellationToken) {
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
    signalled.cancel();
}
