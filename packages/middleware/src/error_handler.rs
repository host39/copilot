//! Maps errors to HTTP responses.
//!
//! Services fail with `DomainError`; handlers return `Result<_, HttpError>` and `?` converts.
//! Every failure body is a `copilot_contracts::ErrorResponse`.
//!
//! | Error | Status |
//! | --- | --- |
//! | `Validation` | 400 |
//! | `Domain` | 400 |
//! | `Forbidden` | 403 |
//! | `EntityNotFound` | 404 |
//! | `BusinessRule` | 422 |
//! | `Internal` | 500 (generic message; details only in logs) |

use std::error::Error as _;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use copilot_contracts::ErrorResponse;
use copilot_core::DomainError;

use crate::request_id;

#[derive(Debug)]
pub enum HttpError {
    /// A domain or infrastructure failure from a service.
    Domain(DomainError),
    /// A transport-level rejection (for example an oversized body) with a ready-made body.
    Status {
        status: StatusCode,
        body: ErrorResponse,
    },
}

impl From<DomainError> for HttpError {
    fn from(error: DomainError) -> Self {
        Self::Domain(error)
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let (status, mut body) = match &self {
            Self::Domain(error) => (status_code(error), format_error(error)),
            Self::Status { status, body } => (*status, body.clone()),
        };
        body.request_id = request_id::current();
        log(&self, status, &body);
        (status, Json(body)).into_response()
    }
}

/// Fallback for unknown routes.
pub async fn not_found() -> Response {
    HttpError::Status {
        status: StatusCode::NOT_FOUND,
        body: ErrorResponse::new("Not found", "NOT_FOUND"),
    }
    .into_response()
}

pub fn status_code(error: &DomainError) -> StatusCode {
    match error {
        DomainError::Forbidden { .. } => StatusCode::FORBIDDEN,
        DomainError::EntityNotFound { .. } => StatusCode::NOT_FOUND,
        DomainError::Validation { .. } | DomainError::Domain { .. } => StatusCode::BAD_REQUEST,
        DomainError::BusinessRule { .. } => StatusCode::UNPROCESSABLE_ENTITY,
        DomainError::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub fn format_error(error: &DomainError) -> ErrorResponse {
    match error {
        DomainError::Forbidden { message, .. } => ErrorResponse {
            error: message.clone(),
            ..ErrorResponse::default()
        },
        DomainError::Validation { message, field, .. } => ErrorResponse {
            error: message.clone(),
            code: Some(error.code().to_owned()),
            field: field.clone(),
            ..ErrorResponse::default()
        },
        DomainError::Domain { message, .. } | DomainError::BusinessRule { message, .. } => {
            ErrorResponse {
                error: message.clone(),
                code: Some(error.code().to_owned()),
                details: Some(error.context()).filter(|context| !context.is_empty()),
                ..ErrorResponse::default()
            }
        }
        DomainError::EntityNotFound { .. } => ErrorResponse {
            error: error.to_string(),
            code: Some(error.code().to_owned()),
            details: Some(error.context()),
            ..ErrorResponse::default()
        },
        DomainError::Internal { .. } => {
            ErrorResponse::new("An unexpected error occurred", "INTERNAL_ERROR")
        }
    }
}

fn log(error: &HttpError, status: StatusCode, body: &ErrorResponse) {
    let status_code = status.as_u16();
    let code = body.code.as_deref();
    let request_id = body.request_id.as_deref();

    if status.is_server_error() {
        let (reason, source) = match error {
            HttpError::Domain(error) => {
                (error.to_string(), error.source().map(ToString::to_string))
            }
            HttpError::Status { body, .. } => (body.error.clone(), None),
        };
        tracing::error!(
            status = status_code,
            code,
            reason,
            source,
            request_id,
            "middleware.error.internal"
        );
    } else {
        tracing::warn!(
            status = status_code,
            code,
            reason = body.error,
            request_id,
            "middleware.error.client"
        );
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn json_body(error: &DomainError) -> Value {
        serde_json::to_value(format_error(error)).expect("serializes")
    }

    #[test]
    fn maps_validation_to_400() {
        let error = DomainError::validation("Invalid email").with_field("email");
        assert_eq!(status_code(&error), StatusCode::BAD_REQUEST);
        assert_eq!(
            json_body(&error),
            json!({ "error": "Invalid email", "code": "VALIDATION_ERROR", "field": "email" })
        );
    }

    #[test]
    fn maps_forbidden_to_403() {
        let error = DomainError::forbidden("Not allowed");
        assert_eq!(status_code(&error), StatusCode::FORBIDDEN);
        assert_eq!(json_body(&error), json!({ "error": "Not allowed" }));
    }

    #[test]
    fn maps_not_found_to_404() {
        let error = DomainError::not_found("Widget", "abc");
        assert_eq!(status_code(&error), StatusCode::NOT_FOUND);
        assert_eq!(
            json_body(&error),
            json!({
                "error": "Widget with id 'abc' not found",
                "code": "ENTITY_NOT_FOUND",
                "details": { "entityName": "Widget", "entityId": "abc" }
            })
        );
    }

    #[test]
    fn maps_business_rule_to_422() {
        let error = DomainError::business_rule("Rule violated", "RULE_VIOLATED");
        assert_eq!(status_code(&error), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            json_body(&error),
            json!({ "error": "Rule violated", "code": "RULE_VIOLATED" })
        );
    }

    #[test]
    fn maps_domain_to_400() {
        let error = DomainError::new("Domain failure", "CUSTOM");
        assert_eq!(status_code(&error), StatusCode::BAD_REQUEST);
        assert_eq!(json_body(&error)["code"], "CUSTOM");
    }

    #[test]
    fn hides_internal_errors() {
        let error = DomainError::internal("database.widgets.insert.failed", "boom");
        assert_eq!(status_code(&error), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            json_body(&error),
            json!({ "error": "An unexpected error occurred", "code": "INTERNAL_ERROR" })
        );
    }
}
