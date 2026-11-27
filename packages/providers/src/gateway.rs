//! [`Gateway`]: the `LlmGateway` port over the injected outbound clients, by wire protocol.

use async_trait::async_trait;
use copilot_core::{
    DomainResult, LlmEvent, LlmGateway, LlmRequest, ModelTarget, Outbound, ProviderKind,
    ProviderTarget, RemoteModel,
};
use futures_util::stream::BoxStream;
use reqwest::Client;

use crate::{anthropic, openai};

/// The code of a request the wire protocol cannot carry (for example audio to the Messages API,
/// or an audio format `input_audio` does not name). Nothing reaches the provider.
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
            ProviderKind::Anthropic => anthropic::models(&self.clients, target).await,
        }
    }

    async fn stream(
        &self,
        target: &ModelTarget,
        request: LlmRequest,
    ) -> DomainResult<BoxStream<'static, DomainResult<LlmEvent>>> {
        match target.provider.kind {
            ProviderKind::OpenAi => openai::stream(&self.clients, target, request).await,
            ProviderKind::Anthropic => anthropic::stream(&self.clients, target, request).await,
        }
    }
}
