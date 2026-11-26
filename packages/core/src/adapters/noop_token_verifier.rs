use async_trait::async_trait;

use crate::ports::token_verifier::{Principal, TokenVerifier};
use crate::shared::errors::DomainResult;

/// Development token verifier: accepts every request when auth is disabled.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopTokenVerifier;

#[async_trait]
impl TokenVerifier for NoopTokenVerifier {
    async fn verify(&self, _header: Option<&str>) -> DomainResult<Principal> {
        Ok(Principal {
            user_id: "dev-user".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn accepts_missing_header() {
        let principal = NoopTokenVerifier.verify(None).await.expect("accepted");
        assert_eq!(principal.user_id, "dev-user");
    }
}
