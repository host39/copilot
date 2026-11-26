//! Port for verifying inbound auth tokens.
//!
//! This template ships without an authentication provider. When a project needs auth, implement
//! this trait (for example against Auth0, Cognito, or a custom JWT issuer) in an adapter crate and
//! pass it to `create_app` in `apps/api`.

use async_trait::async_trait;

use crate::shared::errors::DomainResult;

/// The caller a verified token belongs to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Principal {
    pub user_id: String,
}

#[async_trait]
pub trait TokenVerifier: Send + Sync {
    /// Verifies the raw `Authorization` header value, when present.
    async fn verify(&self, header: Option<&str>) -> DomainResult<Principal>;
}
