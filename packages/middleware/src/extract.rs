//! Request extractors that fail with the shared error body instead of axum's plain text.

use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use copilot_contracts::ErrorResponse;
use copilot_core::DomainError;
use serde::de::DeserializeOwned;

use crate::error_handler::HttpError;

/// JSON request body. Use instead of `axum::Json` in handlers so malformed bodies get a `400`
/// with `code: VALIDATION_ERROR`, oversized bodies a `413`, and a missing content type a `415`.
#[derive(Clone, Copy, Debug, Default)]
pub struct JsonBody<T>(pub T);

impl<S, T> FromRequest<S> for JsonBody<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = HttpError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(request, state).await {
            Ok(axum::Json(value)) => Ok(Self(value)),
            Err(rejection) => Err(reject(&rejection)),
        }
    }
}

fn reject(rejection: &JsonRejection) -> HttpError {
    match rejection.status() {
        StatusCode::PAYLOAD_TOO_LARGE => HttpError::Status {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            body: Box::new(ErrorResponse::new(
                "Request body is too large.",
                "PAYLOAD_TOO_LARGE",
            )),
        },
        StatusCode::UNSUPPORTED_MEDIA_TYPE => HttpError::Status {
            status: StatusCode::UNSUPPORTED_MEDIA_TYPE,
            body: Box::new(ErrorResponse::new(
                "Request body must be JSON (Content-Type: application/json).",
                "UNSUPPORTED_MEDIA_TYPE",
            )),
        },
        _ => HttpError::Domain(DomainError::validation(rejection.body_text())),
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::routing::post;
    use http_body_util::BodyExt;
    use serde::Deserialize;
    use tower::ServiceExt;

    use super::*;

    #[derive(Deserialize)]
    struct Input {
        title: String,
    }

    async fn send(content_type: &str, body: &'static str) -> (StatusCode, serde_json::Value) {
        let app = Router::new().route(
            "/",
            post(|JsonBody(input): JsonBody<Input>| async move { input.title }),
        );
        let request = Request::builder()
            .method("POST")
            .uri("/")
            .header("content-type", content_type)
            .body(Body::from(body))
            .expect("request");
        let response = app.oneshot(request).await.expect("response");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[tokio::test]
    async fn accepts_valid_json() {
        let (status, _) = send("application/json", r#"{"title":"Hello"}"#).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_missing_fields_as_validation_errors() {
        let (status, body) = send("application/json", "{}").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "VALIDATION_ERROR");
    }

    #[tokio::test]
    async fn rejects_other_content_types() {
        let (status, body) = send("text/plain", "{}").await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(body["code"], "UNSUPPORTED_MEDIA_TYPE");
    }
}
