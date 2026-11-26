//! Authentication guard backed by the core `TokenVerifier` port.
//!
//! Wire it with `axum::middleware::from_fn_with_state(verifier, auth)`. Paths in [`PUBLIC_PATHS`]
//! skip verification; every other request must pass the verifier or gets a `401`.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use copilot_contracts::ErrorResponse;
use copilot_core::TokenVerifier;

use crate::request_id;

/// Routes that never require a token.
pub const PUBLIC_PATHS: [&str; 5] = ["/", "/health", "/health/ready", "/doc", "/ui"];

/// The verified caller, available to handlers as `Extension<UserId>`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserId(pub String);

pub async fn auth(
    State(verifier): State<Arc<dyn TokenVerifier>>,
    mut request: Request,
    next: Next,
) -> Response {
    if PUBLIC_PATHS.contains(&request.uri().path()) {
        return next.run(request).await;
    }

    let header = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    match verifier.verify(header.as_deref()).await {
        Ok(principal) => {
            request.extensions_mut().insert(UserId(principal.user_id));
            next.run(request).await
        }
        Err(error) => {
            tracing::warn!(
                path = request.uri().path(),
                reason = %error,
                request_id = request_id::current().as_deref(),
                "middleware.auth.rejected"
            );
            let body = ErrorResponse {
                request_id: request_id::current(),
                ..ErrorResponse::new("Unauthorized", "UNAUTHORIZED")
            };
            (StatusCode::UNAUTHORIZED, Json(body)).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::routing::get;
    use copilot_core::{NoopTokenVerifier, RejectingTokenVerifier};
    use tower::ServiceExt;

    use super::*;

    fn app(verifier: Arc<dyn TokenVerifier>) -> Router {
        Router::new()
            .route("/health", get(|| async { "ok" }))
            .route(
                "/private",
                get(|axum::Extension(user): axum::Extension<UserId>| async move { user.0 }),
            )
            .layer(axum::middleware::from_fn_with_state(verifier, auth))
    }

    async fn status(verifier: Arc<dyn TokenVerifier>, uri: &str) -> StatusCode {
        let request = Request::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("request");
        app(verifier)
            .oneshot(request)
            .await
            .expect("response")
            .status()
    }

    #[tokio::test]
    async fn public_paths_skip_the_verifier() {
        assert_eq!(
            status(Arc::new(RejectingTokenVerifier), "/health").await,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn rejects_when_the_verifier_fails() {
        assert_eq!(
            status(Arc::new(RejectingTokenVerifier), "/private").await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn passes_the_user_to_handlers() {
        assert_eq!(
            status(Arc::new(NoopTokenVerifier), "/private").await,
            StatusCode::OK
        );
    }
}
