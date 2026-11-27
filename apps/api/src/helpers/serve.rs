//! The HTTP server loop: axum's router on hyper's HTTP/1 connections with a timer, so a client
//! that never finishes its request headers is dropped after `HEADER_READ_TIMEOUT_SECS`
//! (`axum::serve` runs hyper without a timer, which turns that timeout off).

use std::future::Future;
use std::pin::pin;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::Request;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tower::ServiceExt;

/// Serves `app` on `listener` until `shutdown` resolves, then stops accepting and waits for the
/// open connections to finish. Each request carries the peer address as
/// `ConnectInfo<SocketAddr>` (lib-rustauth's per-IP limits read it).
pub async fn serve(
    listener: TcpListener,
    app: Router,
    header_timeout: Duration,
    shutdown: impl Future<Output = ()>,
) {
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(header_timeout);
    let graceful = GracefulShutdown::new();
    let mut shutdown = pin!(shutdown);
    loop {
        let (stream, peer) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    // Out of file descriptors, for example: wait a moment instead of spinning.
                    tracing::warn!(error = %error, "api.server.accept.failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            },
            () = &mut shutdown => break,
        };
        let service = app.clone().map_request(move |request: Request<Incoming>| {
            let mut request = request.map(Body::new);
            request.extensions_mut().insert(ConnectInfo(peer));
            request
        });
        let connection =
            builder.serve_connection(TokioIo::new(stream), TowerToHyperService::new(service));
        let connection = graceful.watch(connection);
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::debug!(error = %error, peer = %peer, "api.server.connection.failed");
            }
        });
    }
    drop(listener);
    graceful.shutdown().await;
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::time::Instant;

    use axum::routing::get;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio_util::sync::CancellationToken;

    use super::*;

    async fn start(header_timeout: Duration) -> (SocketAddr, CancellationToken) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/peer",
            get(|ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { peer.ip().to_string() }),
        );
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        tokio::spawn(async move {
            serve(listener, app, header_timeout, stopped.cancelled_owned()).await;
        });
        (address, stop)
    }

    #[tokio::test]
    async fn answers_with_the_peer_address() {
        let (address, stop) = start(Duration::from_secs(5)).await;
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream
            .write_all(b"GET /peer HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).await.unwrap();
        assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
        assert!(answer.ends_with("127.0.0.1"), "{answer}");
        stop.cancel();
    }

    #[tokio::test]
    async fn drops_a_client_that_never_finishes_its_headers() {
        let (address, stop) = start(Duration::from_millis(300)).await;
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream
            .write_all(b"GET /peer HTTP/1.1\r\nHost: x\r\n")
            .await
            .unwrap();
        let started = Instant::now();
        let mut answer = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut answer))
            .await
            .expect("the server closes the connection");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(
            answer.is_empty() || answer.starts_with(b"HTTP/1.1 408"),
            "{}",
            String::from_utf8_lossy(&answer)
        );
        stop.cancel();
    }

    #[tokio::test]
    async fn stops_accepting_on_shutdown() {
        let (address, stop) = start(Duration::from_secs(5)).await;
        stop.cancel();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(TcpStream::connect(address).await.is_err());
    }
}
