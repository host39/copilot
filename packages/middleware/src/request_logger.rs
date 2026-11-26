//! One structured log line per request. The query string is logged as its parameter names only
//! (`query = "q,limit"`): values such as search text come from users' conversations, and
//! prompts and similar content never reach the logs.

use std::time::Instant;

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;

use crate::request_id;

#[allow(
    clippy::cognitive_complexity,
    reason = "each tracing macro expands into several branches; the match itself is flat"
)]
pub async fn request_logger(request: Request, next: Next) -> Response {
    let method = request.method().to_string();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().map(names);
    let start = Instant::now();

    let response = next.run(request).await;

    let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    let status = response.status().as_u16();
    let request_id = request_id::current();
    let query = query.as_deref();
    let request_id = request_id.as_deref();

    match status {
        413 => tracing::warn!(
            method,
            path,
            status,
            duration_ms,
            query,
            request_id,
            "middleware.http.body-too-large"
        ),
        500.. => tracing::error!(
            method,
            path,
            status,
            duration_ms,
            query,
            request_id,
            "middleware.http.failed"
        ),
        400.. => tracing::warn!(
            method,
            path,
            status,
            duration_ms,
            query,
            request_id,
            "middleware.http.client-error"
        ),
        _ => tracing::info!(
            method,
            path,
            status,
            duration_ms,
            query,
            request_id,
            "middleware.http.ok"
        ),
    }

    response
}

/// Longest list of parameter names logged.
const NAMES_MAX_CHARS: usize = 200;

/// The parameter names of a query string, comma-separated and cut to `NAMES_MAX_CHARS`.
fn names(query: &str) -> String {
    let names = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| pair.split_once('=').map_or(pair, |(name, _)| name))
        .collect::<Vec<_>>()
        .join(",");
    names.chars().take(NAMES_MAX_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware::from_fn;
    use axum::routing::get;
    use tower::ServiceExt;

    use super::*;

    /// Collects formatted log lines.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("log lock").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn names_keep_parameter_names_only() {
        assert_eq!(names("q=my%20secret&limit=5"), "q,limit");
        assert_eq!(names("flag&&q="), "flag,q");
        assert_eq!(names(&"a".repeat(500)).len(), NAMES_MAX_CHARS);
    }

    #[tokio::test]
    async fn a_search_is_logged_without_its_text() {
        let logs = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer({
                let logs = logs.clone();
                move || logs.clone()
            })
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let app = Router::new()
            .route("/api/search", get(|| async { StatusCode::OK }))
            .layer(from_fn(request_logger));
        let response = app
            .oneshot(
                Request::get("/api/search?q=secret%20diagnosis&limit=5")
                    .body(Body::empty())
                    .expect("a request"),
            )
            .await
            .expect("a response");
        assert_eq!(response.status(), StatusCode::OK);

        let text = String::from_utf8(logs.0.lock().expect("log lock").clone()).expect("UTF-8");
        assert!(text.contains("middleware.http.ok"), "{text}");
        assert!(text.contains("q,limit"), "{text}");
        assert!(!text.contains("secret"), "{text}");
        assert!(!text.contains("diagnosis"), "{text}");
    }
}
