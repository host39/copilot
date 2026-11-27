//! Path and query parameters that fail with the shared error body instead of axum's plain text.

use axum::extract::{FromRequestParts, Path};
use axum::http::request::Parts;
use copilot_core::DomainError;
use copilot_middleware::HttpError;
use serde::de::DeserializeOwned;
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

/// The `{id}` and `{messageId}` of a message route. A value that is not a UUID is a `400`
/// (`VALIDATION_ERROR`, field `id` or `messageId`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessageIds {
    pub chat: Uuid,
    pub message: Uuid,
}

impl<S: Send + Sync> FromRequestParts<S> for MessageIds {
    type Rejection = HttpError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let invalid = |field: &str| {
            HttpError::Domain(
                DomainError::validation("The id in the path is not valid.").with_field(field),
            )
        };
        let Ok(Path((chat, message))) =
            Path::<(String, String)>::from_request_parts(parts, state).await
        else {
            return Err(invalid("id"));
        };
        Ok(Self {
            chat: chat.parse().map_err(|_| invalid("id"))?,
            message: message.parse().map_err(|_| invalid("messageId"))?,
        })
    }
}

/// The query string of a route, read into `T`. A value that does not fit is a `400`
/// (`VALIDATION_ERROR`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Params<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequestParts<S> for Params<T> {
    type Rejection = HttpError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Query::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Query(value)) => Ok(Self(value)),
            Err(_) => Err(HttpError::Domain(DomainError::validation(
                "The query string is not valid.",
            ))),
        }
    }
}
