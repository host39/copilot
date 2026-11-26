//! Request id: reuse the caller's `X-Request-Id` or generate one, expose it to logs and error
//! bodies for the rest of the request, and echo it on the response.

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use uuid::Uuid;

pub const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

/// Longest caller-supplied id we accept; longer values are replaced with a fresh one.
const MAX_LENGTH: usize = 128;

tokio::task_local! {
    static CURRENT: String;
}

/// The id of the current request, available to handlers as a request extension.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestId(pub String);

pub async fn request_id(mut request: Request, next: Next) -> Response {
    let id = request
        .headers()
        .get(&REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty() && value.len() <= MAX_LENGTH)
        .map_or_else(|| Uuid::new_v4().to_string(), str::to_owned);

    request.extensions_mut().insert(RequestId(id.clone()));
    let mut response = CURRENT.scope(id.clone(), next.run(request)).await;

    if let Ok(value) = HeaderValue::from_str(&id) {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    response
}

/// The current request id, when called inside a request handled by [`request_id`].
pub fn current() -> Option<String> {
    CURRENT.try_with(Clone::clone).ok()
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::routing::get;
    use tower::ServiceExt;

    use super::*;

    fn app() -> Router {
        Router::new()
            .route("/", get(|| async { current().unwrap_or_default() }))
            .layer(axum::middleware::from_fn(request_id))
    }

    #[tokio::test]
    async fn echoes_the_incoming_header() {
        let request = Request::builder()
            .uri("/")
            .header(REQUEST_ID_HEADER, "ray-1")
            .body(Body::empty())
            .expect("request");
        let response = app().oneshot(request).await.expect("response");
        assert_eq!(response.headers()[REQUEST_ID_HEADER], "ray-1");
    }

    #[tokio::test]
    async fn generates_an_id_when_missing() {
        let request = Request::builder()
            .uri("/")
            .body(Body::empty())
            .expect("request");
        let response = app().oneshot(request).await.expect("response");
        let header = response.headers()[REQUEST_ID_HEADER]
            .to_str()
            .expect("ascii");
        assert!(Uuid::parse_str(header).is_ok());
    }
}
