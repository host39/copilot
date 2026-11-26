//! Stand-ins for the provider and MCP adapters until Phase 2 tracks B and C build them
//! (`copilot-providers`, `copilot-mcp`). The integration step P2.F replaces them in the
//! composition root; nothing else names them. Every call fails with a human-readable
//! [`DomainError::Upstream`] (`502`), so a refresh, a test, or a turn reports that the feature is
//! not there yet instead of failing silently.

use async_trait::async_trait;
use copilot_core::{
    DomainError, DomainResult, LlmEvent, LlmGateway, LlmRequest, ModelTarget, ProviderTarget,
    RemoteModel, ToolEndpoint, ToolHost, ToolListing, ToolSession,
};
use futures_util::stream::BoxStream;

/// The code of every failure here.
pub const UNAVAILABLE: &str = "ADAPTER_UNAVAILABLE";

/// Implements [`LlmGateway`] and [`ToolHost`] by refusing every call.
#[derive(Clone, Copy, Debug, Default)]
pub struct Unavailable;

fn providers() -> DomainError {
    DomainError::upstream(
        "Model providers are not connected in this version yet.",
        UNAVAILABLE,
    )
}

fn tools() -> DomainError {
    DomainError::upstream(
        "Tool servers are not connected in this version yet.",
        UNAVAILABLE,
    )
}

#[async_trait]
impl LlmGateway for Unavailable {
    async fn models(&self, _target: &ProviderTarget) -> DomainResult<Vec<RemoteModel>> {
        Err(providers())
    }

    async fn stream(
        &self,
        _target: &ModelTarget,
        _request: LlmRequest,
    ) -> DomainResult<BoxStream<'static, DomainResult<LlmEvent>>> {
        Err(providers())
    }
}

#[async_trait]
impl ToolHost for Unavailable {
    async fn list(&self, _endpoint: &ToolEndpoint) -> DomainResult<ToolListing> {
        Err(tools())
    }

    async fn open(&self, _endpoints: Vec<ToolEndpoint>) -> DomainResult<Box<dyn ToolSession>> {
        Err(tools())
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use copilot_middleware::error_handler::status_code;

    use super::*;

    #[tokio::test]
    async fn tool_sessions_fail_with_a_readable_upstream_error() {
        let Err(error) = ToolHost::open(&Unavailable, Vec::new()).await else {
            panic!("refused");
        };
        assert_eq!(error.code(), UNAVAILABLE);
        assert_eq!(status_code(&error), StatusCode::BAD_GATEWAY);
        assert_eq!(
            error.to_string(),
            "Tool servers are not connected in this version yet."
        );
    }

    #[test]
    fn provider_failures_name_the_providers() {
        let error = providers();
        assert_eq!(error.code(), UNAVAILABLE);
        assert_eq!(status_code(&error), StatusCode::BAD_GATEWAY);
        assert!(error.to_string().starts_with("Model providers"));
    }
}
