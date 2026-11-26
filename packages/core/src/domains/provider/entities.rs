//! Provider connections and their models.

use chrono::{DateTime, Utc};

use crate::domains::provider::capabilities::{Capabilities, CapabilityOverrides, Locks};
use crate::domains::provider::values::{Dialect, Preset, ProviderKind};
use crate::ports::llm_gateway::{ModelTarget, ProviderTarget};
use crate::ports::secret_cipher::SealContext;
use crate::shared::base_url::BaseUrl;
use crate::shared::constants::NAME_MAX_CHARS;
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{ModelId, ProviderId, UserId};
use crate::shared::net::NetworkScope;
use crate::shared::secret::Secret;
use crate::shared::text;

/// A user's connection to a model provider (`providers`). The API key is stored sealed and bound
/// to this row ([`Provider::key_context`]).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Provider {
    id: ProviderId,
    user_id: UserId,
    kind: ProviderKind,
    preset: Preset,
    name: String,
    base_url: BaseUrl,
    api_key_sealed: Option<Vec<u8>>,
    enabled: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Every field of a stored provider, for [`Provider::restore`].
#[derive(Clone, Debug)]
pub struct ProviderParts {
    pub id: ProviderId,
    pub user_id: UserId,
    pub kind: ProviderKind,
    pub preset: Preset,
    pub name: String,
    pub base_url: BaseUrl,
    pub api_key_sealed: Option<Vec<u8>>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Whether a provider's key can be used.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyState {
    /// No key stored.
    Missing,
    /// A key is stored and opens.
    Set,
    /// A key is stored but no longer opens (the `SECRET_KEY` changed): "Key must be re-entered".
    Unreadable,
}

impl Provider {
    /// A new, enabled connection without a key. Validates the name and that the preset exists
    /// for the kind.
    pub fn create(
        user_id: UserId,
        kind: ProviderKind,
        preset: Preset,
        name: &str,
        base_url: BaseUrl,
        now: DateTime<Utc>,
    ) -> DomainResult<Self> {
        if !preset.fits(kind) {
            return Err(DomainError::validation(format!(
                "The {preset} preset does not exist for {kind} connections."
            ))
            .with_field("preset"));
        }
        Ok(Self {
            id: ProviderId::generate(),
            user_id,
            kind,
            preset,
            name: provider_name(name)?,
            base_url,
            api_key_sealed: None,
            enabled: true,
            created_at: now,
            updated_at: now,
        })
    }

    /// A row read back from storage.
    pub fn restore(parts: ProviderParts) -> Self {
        Self {
            id: parts.id,
            user_id: parts.user_id,
            kind: parts.kind,
            preset: parts.preset,
            name: parts.name,
            base_url: parts.base_url,
            api_key_sealed: parts.api_key_sealed,
            enabled: parts.enabled,
            created_at: parts.created_at,
            updated_at: parts.updated_at,
        }
    }

    pub fn with_name(mut self, name: &str) -> DomainResult<Self> {
        self.name = provider_name(name)?;
        Ok(self)
    }

    #[must_use]
    pub fn with_base_url(mut self, base_url: BaseUrl) -> Self {
        self.base_url = base_url;
        self
    }

    /// Sets or clears the sealed key. Seal it with [`Provider::key_context`].
    #[must_use]
    pub fn with_key(mut self, sealed: Option<Vec<u8>>) -> Self {
        self.api_key_sealed = sealed;
        self
    }

    #[must_use]
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    #[must_use]
    pub fn touched(mut self, now: DateTime<Utc>) -> Self {
        self.updated_at = now;
        self
    }

    /// The associated data that binds the sealed key to this row.
    pub fn key_context(&self) -> SealContext {
        SealContext {
            table: "providers",
            column: "api_key_sealed",
            row_id: self.id.uuid(),
            user_id: self.user_id,
        }
    }

    pub fn dialect(&self) -> Dialect {
        Dialect::new(self.kind, self.preset)
    }

    /// The adapter's view of this connection, with the opened key.
    pub fn target(&self, api_key: Option<Secret>, network: NetworkScope) -> ProviderTarget {
        ProviderTarget {
            provider_id: self.id,
            kind: self.kind,
            preset: self.preset,
            base_url: self.base_url.clone(),
            api_key,
            network,
        }
    }

    pub fn id(&self) -> ProviderId {
        self.id
    }

    pub fn user_id(&self) -> UserId {
        self.user_id
    }

    pub fn kind(&self) -> ProviderKind {
        self.kind
    }

    pub fn preset(&self) -> Preset {
        self.preset
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn base_url(&self) -> &BaseUrl {
        &self.base_url
    }

    pub fn api_key_sealed(&self) -> Option<&[u8]> {
        self.api_key_sealed.as_deref()
    }

    pub fn has_key(&self) -> bool {
        self.api_key_sealed.is_some()
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

fn provider_name(name: &str) -> DomainResult<String> {
    text::name(name, "Provider name", "name", NAME_MAX_CHARS)
}

/// A model of a provider connection (`models`). Models are created by a refresh
/// (`ModelRepository::sync`), never deleted by one: a model the provider no longer lists becomes
/// unavailable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Model {
    id: ModelId,
    user_id: UserId,
    provider_id: ProviderId,
    remote_id: String,
    display_name: String,
    enabled: bool,
    available: bool,
    context_window: Option<u32>,
    max_output_tokens: Option<u32>,
    capabilities: Capabilities,
    overrides: CapabilityOverrides,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Every field of a stored model, for [`Model::restore`].
#[derive(Clone, Debug)]
pub struct ModelParts {
    pub id: ModelId,
    pub user_id: UserId,
    pub provider_id: ProviderId,
    pub remote_id: String,
    pub display_name: String,
    pub enabled: bool,
    pub available: bool,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    /// Detected capabilities (defaults and locks applied at refresh).
    pub capabilities: Capabilities,
    pub overrides: CapabilityOverrides,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Model {
    /// A row read back from storage (or created by a refresh).
    pub fn restore(parts: ModelParts) -> Self {
        Self {
            id: parts.id,
            user_id: parts.user_id,
            provider_id: parts.provider_id,
            remote_id: parts.remote_id,
            display_name: parts.display_name,
            enabled: parts.enabled,
            available: parts.available,
            context_window: parts.context_window,
            max_output_tokens: parts.max_output_tokens,
            capabilities: parts.capabilities,
            overrides: parts.overrides,
            created_at: parts.created_at,
            updated_at: parts.updated_at,
        }
    }

    #[must_use]
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    #[must_use]
    pub fn with_display_name(mut self, display_name: String) -> Self {
        self.display_name = display_name;
        self
    }

    #[must_use]
    pub fn with_overrides(mut self, overrides: CapabilityOverrides) -> Self {
        self.overrides = overrides;
        self
    }

    #[must_use]
    pub fn with_limits(
        mut self,
        context_window: Option<u32>,
        max_output_tokens: Option<u32>,
    ) -> Self {
        self.context_window = context_window;
        self.max_output_tokens = max_output_tokens;
        self
    }

    #[must_use]
    pub fn touched(mut self, now: DateTime<Utc>) -> Self {
        self.updated_at = now;
        self
    }

    /// The locked flags of this model on `provider`.
    pub fn locks(&self, provider: &Provider) -> Locks {
        Locks::new(provider.kind(), provider.preset(), &self.remote_id)
    }

    /// What the model can do once overrides and locks apply.
    pub fn effective(&self, provider: &Provider) -> Capabilities {
        Capabilities::effective(&self.capabilities, &self.overrides, &self.locks(provider))
    }

    /// The adapter's view of this model.
    pub fn target(&self, provider: ProviderTarget, dialect: Dialect) -> ModelTarget {
        ModelTarget {
            provider,
            remote_id: self.remote_id.clone(),
            dialect,
        }
    }

    /// Enabled by the user and still listed by the provider: the picker shows it.
    pub fn is_usable(&self) -> bool {
        self.enabled && self.available
    }

    pub fn id(&self) -> ModelId {
        self.id
    }

    pub fn user_id(&self) -> UserId {
        self.user_id
    }

    pub fn provider_id(&self) -> ProviderId {
        self.provider_id
    }

    pub fn remote_id(&self) -> &str {
        &self.remote_id
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn available(&self) -> bool {
        self.available
    }

    pub fn context_window(&self) -> Option<u32> {
        self.context_window
    }

    pub fn max_output_tokens(&self) -> Option<u32> {
        self.max_output_tokens
    }

    /// Detected capabilities, before overrides and locks.
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    pub fn overrides(&self) -> &CapabilityOverrides {
        &self.overrides
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use uuid::Uuid;

    use super::*;
    use crate::domains::provider::capabilities::Capability;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    fn url(text: &str) -> BaseUrl {
        BaseUrl::parse(text, NetworkScope::Private).unwrap()
    }

    fn user() -> UserId {
        UserId::new(Uuid::now_v7())
    }

    #[test]
    fn creates_an_enabled_connection_without_a_key() {
        let provider = Provider::create(
            user(),
            ProviderKind::Anthropic,
            Preset::Anthropic,
            " Claude ",
            url("https://api.anthropic.com"),
            now(),
        )
        .unwrap();
        assert_eq!(provider.name(), "Claude");
        assert!(provider.enabled());
        assert!(!provider.has_key());
        assert_eq!(provider.dialect(), Dialect::Anthropic);
        assert_eq!(provider.created_at(), provider.updated_at());
    }

    #[test]
    fn rejects_a_preset_of_another_kind() {
        let error = Provider::create(
            user(),
            ProviderKind::Anthropic,
            Preset::Ollama,
            "Local",
            url("http://localhost:11434/v1"),
            now(),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "The ollama preset does not exist for anthropic connections."
        );
    }

    #[test]
    fn binds_the_key_to_the_row_and_owner() {
        let owner = user();
        let provider = Provider::create(
            owner,
            ProviderKind::OpenAi,
            Preset::OpenAi,
            "OpenAI",
            url("https://api.openai.com/v1"),
            now(),
        )
        .unwrap();
        let context = provider.key_context();
        assert_eq!(context.table, "providers");
        assert_eq!(context.column, "api_key_sealed");
        assert_eq!(context.row_id, provider.id().uuid());
        assert_eq!(context.user_id, owner);
    }

    #[test]
    fn computes_effective_capabilities_with_the_connection_locks() {
        let provider = Provider::create(
            user(),
            ProviderKind::Anthropic,
            Preset::Anthropic,
            "Claude",
            url("https://api.anthropic.com"),
            now(),
        )
        .unwrap();
        let model = Model::restore(ModelParts {
            id: ModelId::generate(),
            user_id: provider.user_id(),
            provider_id: provider.id(),
            remote_id: "claude-opus-5-5".to_owned(),
            display_name: "Claude Opus 5.5".to_owned(),
            enabled: true,
            available: false,
            context_window: Some(1_000_000),
            max_output_tokens: Some(128_000),
            capabilities: Capabilities {
                tools: true,
                audio_in: true,
                ..Capabilities::default()
            },
            overrides: CapabilityOverrides::default(),
            created_at: now(),
            updated_at: now(),
        });
        assert!(model.locks(&provider).contains(Capability::AudioIn));
        assert!(!model.effective(&provider).audio_in);
        assert!(model.effective(&provider).tools);
        assert!(!model.is_usable(), "an unavailable model is not usable");
        let target = model.target(
            provider.target(None, NetworkScope::Public),
            provider.dialect(),
        );
        assert_eq!(target.remote_id, "claude-opus-5-5");
        assert_eq!(target.dialect, Dialect::Anthropic);
    }
}
