//! [`Gateway`]: the `LlmGateway` port over the injected outbound clients, by wire protocol.

use async_trait::async_trait;
use copilot_core::{
    DomainError, DomainResult, LlmEvent, LlmGateway, LlmRequest, ModelTarget, Outbound,
    ProviderKind, ProviderTarget, RemoteModel,
};
use futures_util::stream::BoxStream;
use reqwest::Client;

use crate::openai;

/// The code of a request this version cannot send yet (a protocol or a part type that Phase 2
/// track B adds). Nothing reaches the provider.
pub const UNSUPPORTED: &str = "PROVIDER_UNSUPPORTED";

/// Every provider connection, through the three hardened clients the composition root builds
/// with `copilot_security::clients(Purpose::Providers)`. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Gateway {
    clients: Outbound<Client>,
}

impl Gateway {
    pub fn new(clients: Outbound<Client>) -> Self {
        Self { clients }
    }
}

#[async_trait]
impl LlmGateway for Gateway {
    async fn models(&self, target: &ProviderTarget) -> DomainResult<Vec<RemoteModel>> {
        match target.kind {
            ProviderKind::OpenAi => openai::models(&self.clients, target).await,
            ProviderKind::Anthropic => Err(anthropic()),
        }
    }

    async fn stream(
        &self,
        target: &ModelTarget,
        request: LlmRequest,
    ) -> DomainResult<BoxStream<'static, DomainResult<LlmEvent>>> {
        match target.provider.kind {
            ProviderKind::OpenAi => openai::stream(&self.clients, target, request).await,
            ProviderKind::Anthropic => Err(anthropic()),
        }
    }
}

fn anthropic() -> DomainError {
    DomainError::upstream(
        "Anthropic connections are not supported in this version yet.",
        UNSUPPORTED,
    )
}
