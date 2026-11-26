//! Model provider port.
//!
//! `copilot-providers` implements [`LlmGateway`] for the OpenAI-compatible and Anthropic wire
//! protocols. Core builds every target, so adapters never read the database or the cipher.
//!
//! This step defines model listing. P1.3 (the conversation engine) adds
//! `stream(&ModelTarget, LlmRequest)` with `LlmRequest` and `LlmEvent`, and then freezes the port.

use async_trait::async_trait;

use crate::domains::provider::{Dialect, Preset, ProviderKind};
use crate::shared::base_url::BaseUrl;
use crate::shared::errors::DomainResult;
use crate::shared::ids::ProviderId;
use crate::shared::net::NetworkScope;
use crate::shared::reasoning::ReasoningLevel;
use crate::shared::secret::Secret;

#[async_trait]
pub trait LlmGateway: Send + Sync {
    /// The provider's models. Adapters map HTTP failures to human-readable `DomainError`s
    /// (for example `PROVIDER_AUTH` "Key rejected: open provider settings.").
    async fn models(&self, target: &ProviderTarget) -> DomainResult<Vec<RemoteModel>>;
}

/// Everything an adapter needs to reach a provider connection. `network` picks the outbound
/// client (public-only or private-allowed).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderTarget {
    pub provider_id: ProviderId,
    pub kind: ProviderKind,
    pub preset: Preset,
    pub base_url: BaseUrl,
    pub api_key: Option<Secret>,
    pub network: NetworkScope,
}

/// A model on a provider connection, with the dialect for its requests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelTarget {
    pub provider: ProviderTarget,
    pub remote_id: String,
    pub dialect: Dialect,
}

/// A model as the provider lists it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RemoteModel {
    pub remote_id: String,
    pub display_name: String,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reported: ReportedCapabilities,
}

/// Capabilities the provider reported (Anthropic Models API `capabilities`, OpenRouter
/// `architecture` modalities). `None` means "not reported"; core fills the gaps with preset
/// defaults (`Capabilities::defaults`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReportedCapabilities {
    pub vision: Option<bool>,
    pub pdf: Option<bool>,
    pub tools: Option<bool>,
    pub audio_in: Option<bool>,
    pub audio_out: Option<bool>,
    pub reasoning: Option<bool>,
    pub reasoning_levels: Option<Vec<ReasoningLevel>>,
    pub default_reasoning: Option<ReasoningLevel>,
}
