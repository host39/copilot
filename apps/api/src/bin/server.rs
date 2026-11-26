//! Long-running HTTP server (the default entrypoint).

use std::net::SocketAddr;
use std::process::ExitCode;

use copilot_api::helpers::bootstrap::ApiContext;
use copilot_core::BoxError;
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

    // The peer address feeds lib-rustauth's per-IP limits and `AUTH_TRUSTED_PROXIES`.
    let app = context
        .app
        .clone()
        .into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown(context.shutdown.clone()))
        .await?;

    context.close().await;
    tracing::info!("api.server.stopped");
    Ok(())
}

/// Resolves on Ctrl+C or SIGTERM so in-flight requests can finish, after cancelling `root` so
/// open streams end and runs stop instead of holding the shutdown open.
async fn shutdown(root: CancellationToken) {
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
    root.cancel();
}
