use async_trait::async_trait;

use crate::ports::token_verifier::{Principal, TokenVerifier};
use crate::shared::errors::{DomainError, DomainResult};

/// Placeholder verifier when auth is enabled but no provider is wired yet.
/// Rejects every request until swapped for a real implementation.
#[derive(Clone, Copy, Debug, Default)]
pub struct RejectingTokenVerifier;

#[async_trait]
impl TokenVerifier for RejectingTokenVerifier {
    async fn verify(&self, _header: Option<&str>) -> DomainResult<Principal> {
        Err(DomainError::new("Unauthorized", "UNAUTHORIZED"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_every_header() {
        let result = RejectingTokenVerifier.verify(Some("Bearer token")).await;
        assert!(result.is_err());
    }
}
