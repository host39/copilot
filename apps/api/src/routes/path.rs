//! Path parameters that fail with the shared error body instead of axum's plain text.

use axum::extract::{FromRequestParts, Path};
use axum::http::request::Parts;
use copilot_core::DomainError;
use copilot_middleware::HttpError;
use uuid::Uuid;

/// The `{id}` of a route. A value that is not a UUID is a `400` (`VALIDATION_ERROR`, field `id`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Id(pub Uuid);

impl<S: Send + Sync> FromRequestParts<S> for Id {
    type Rejection = HttpError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Path::<Uuid>::from_request_parts(parts, state).await {
            Ok(Path(id)) => Ok(Self(id)),
            Err(_) => Err(HttpError::Domain(
                DomainError::validation("The id in the path is not valid.").with_field("id"),
            )),
        }
    }
}
