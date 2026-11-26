//! One structured log line per request.

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
    let query = request.uri().query().map(str::to_owned);
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
