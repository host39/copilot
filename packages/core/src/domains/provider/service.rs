//! Provider connection use cases.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use crate::domains::provider::capabilities::{Capabilities, Locks};
use crate::domains::provider::entities::{KeyState, Provider};
use crate::domains::provider::ports::{
    DetectedModel, ModelQuery, ModelRepository, ProviderRepository, ProviderUsage, SyncReport,
};
use crate::domains::provider::values::{Preset, ProviderKind};
use crate::domains::schedule::{MODEL_REMOVED, ScheduleRepository};
use crate::ports::clock::Clock;
use crate::ports::llm_gateway::{LlmGateway, ProviderTarget};
use crate::ports::secret_cipher::SecretCipher;
use crate::shared::actor::Actor;
use crate::shared::base_url::BaseUrl;
use crate::shared::constants::{
    MAX_MODELS_PER_PROVIDER, MODEL_AUTO_ENABLE_MAX, MODEL_ID_MAX_CHARS, NAME_MAX_CHARS,
};
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::ProviderId;
use crate::shared::net::PrivateNetwork;
use crate::shared::secret::REDACTED;

/// Input of [`ProviderService::create`]. `Debug` prints `***` for the key.
#[derive(Clone)]
pub struct NewProvider {
    pub kind: ProviderKind,
    pub preset: Preset,
    pub name: String,
    /// `None` takes the preset's base URL (`custom` requires one).
    pub base_url: Option<String>,
    /// Blank means no key.
    pub api_key: Option<String>,
}

impl fmt::Debug for NewProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NewProvider")
            .field("kind", &self.kind)
            .field("preset", &self.preset)
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| REDACTED))
            .finish()
    }
}

/// Input of [`ProviderService::update`]; `None` keeps a field. Kind and preset never change.
#[derive(Clone, Debug, Default)]
pub struct ProviderUpdate {
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<KeyChange>,
    pub enabled: Option<bool>,
}

/// A change to the stored API key. `Debug` prints `***` for the key.
#[derive(Clone, Eq, PartialEq)]
pub enum KeyChange {
    /// A new key; blank clears it.
    Set(String),
    Clear,
}

impl fmt::Debug for KeyChange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Set(_) => formatter.debug_tuple("Set").field(&REDACTED).finish(),
            Self::Clear => formatter.write_str("Clear"),
        }
    }
}

/// A connection with what its settings page and delete confirmation show.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderDetails {
    pub provider: Provider,
    pub usage: ProviderUsage,
    pub key: KeyState,
}

#[derive(Clone)]
pub struct ProviderService {
    providers: Arc<dyn ProviderRepository>,
    models: Arc<dyn ModelRepository>,
    schedules: Arc<dyn ScheduleRepository>,
    gateway: Arc<dyn LlmGateway>,
    cipher: Arc<dyn SecretCipher>,
    clock: Arc<dyn Clock>,
    network: PrivateNetwork,
}

impl ProviderService {
    /// `network` is `ALLOW_PRIVATE_NETWORK`.
    pub fn new(
        providers: Arc<dyn ProviderRepository>,
        models: Arc<dyn ModelRepository>,
        schedules: Arc<dyn ScheduleRepository>,
        gateway: Arc<dyn LlmGateway>,
        cipher: Arc<dyn SecretCipher>,
        clock: Arc<dyn Clock>,
        network: PrivateNetwork,
    ) -> Self {
        Self {
            providers,
            models,
            schedules,
            gateway,
            cipher,
            clock,
            network,
        }
    }

    pub async fn list(&self, actor: &Actor) -> DomainResult<Vec<Provider>> {
        self.providers.list(actor.user_id).await
    }

    /// The connection, its usage counts, and whether its key still opens.
    pub async fn get(&self, actor: &Actor, id: ProviderId) -> DomainResult<ProviderDetails> {
        let provider = self.load(actor, id).await?;
        let usage = self.providers.usage(actor.user_id, id).await?;
        let key = KeyState::check(
            &*self.cipher,
            provider.api_key_sealed(),
            &provider.key_context(),
        )?;
        Ok(ProviderDetails {
            provider,
            usage,
            key,
        })
    }

    pub async fn create(&self, actor: &Actor, input: NewProvider) -> DomainResult<Provider> {
        let base_url = match input.base_url.as_deref().or(input.preset.base_url()) {
            Some(text) => self.base_url(actor, text)?,
            None => {
                return Err(
                    DomainError::validation("Enter the base URL of this connection.")
                        .with_field("baseUrl"),
                );
            }
        };
        let provider = Provider::create(
            actor.user_id,
            input.kind,
            input.preset,
            &input.name,
            base_url,
            self.clock.now(),
        )?;
        let sealed = self.seal(&provider, input.api_key.as_deref())?;
        let provider = provider.with_key(sealed);
        self.providers.insert(&provider).await?;
        Ok(provider)
    }

    /// Changing the origin (scheme, host, or port) of the base URL clears the stored key unless
    /// the same update sets a new one, so a key never goes to a host it was not entered for.
    pub async fn update(
        &self,
        actor: &Actor,
        id: ProviderId,
        changes: ProviderUpdate,
    ) -> DomainResult<Provider> {
        let mut provider = self.load(actor, id).await?;
        if let Some(name) = &changes.name {
            provider = provider.with_name(name)?;
        }
        let mut key = changes.api_key;
        if let Some(text) = &changes.base_url {
            let base_url = self.base_url(actor, text)?;
            if !base_url.same_origin(provider.base_url()) && key.is_none() {
                key = Some(KeyChange::Clear);
            }
            provider = provider.with_base_url(base_url);
        }
        match key {
            Some(KeyChange::Set(text)) => {
                let sealed = self.seal(&provider, Some(&text))?;
                provider = provider.with_key(sealed);
            }
            Some(KeyChange::Clear) => provider = provider.with_key(None),
            None => {}
        }
        if let Some(enabled) = changes.enabled {
            provider = provider.with_enabled(enabled);
        }
        let provider = provider.touched(self.clock.now());
        self.providers.update(&provider).await?;
        Ok(provider)
    }

    /// Deletes the connection and its models. Schedules that run one of its models are paused
    /// first ("Model removed"); chats, projects, and schedules that used one keep existing with
    /// no model (`ProviderDetails::usage` counts them first).
    pub async fn delete(&self, actor: &Actor, id: ProviderId) -> DomainResult<()> {
        self.load(actor, id).await?;
        let models = self
            .models
            .list(actor.user_id, ModelQuery::provider(id))
            .await?;
        for model in &models {
            self.schedules
                .disable_for_model(actor.user_id, model.id(), MODEL_REMOVED)
                .await?;
        }
        if self.providers.delete(actor.user_id, id).await? {
            Ok(())
        } else {
            Err(not_found(id))
        }
    }

    /// Lists the provider's models (this is also the connection test), applies capability
    /// defaults and locks, and stores the result. New models are enabled automatically when the
    /// provider lists at most `MODEL_AUTO_ENABLE_MAX` models. A list of more than
    /// `MAX_MODELS_PER_PROVIDER` models, or a model id longer than `MODEL_ID_MAX_CHARS`, fails
    /// as a bad response and stores nothing; display names are cut to `NAME_MAX_CHARS`.
    pub async fn refresh(&self, actor: &Actor, id: ProviderId) -> DomainResult<SyncReport> {
        let provider = self.load(actor, id).await?;
        let target = self.target(actor, &provider)?;
        let listed = self.gateway.models(&target).await?;
        if listed.len() > MAX_MODELS_PER_PROVIDER {
            return Err(DomainError::upstream(
                format!(
                    "The provider listed more than {MAX_MODELS_PER_PROVIDER} models, more than \
                     a connection can keep."
                ),
                BAD_LIST,
            ));
        }
        if listed
            .iter()
            .any(|remote| remote.remote_id.trim().chars().count() > MODEL_ID_MAX_CHARS)
        {
            return Err(DomainError::upstream(
                format!(
                    "The provider listed a model id longer than {MODEL_ID_MAX_CHARS} characters."
                ),
                BAD_LIST,
            ));
        }

        let mut seen = HashSet::new();
        let detected: Vec<DetectedModel> = listed
            .into_iter()
            .filter_map(|mut remote| {
                remote.remote_id = remote.remote_id.trim().to_owned();
                if remote.remote_id.is_empty() || !seen.insert(remote.remote_id.clone()) {
                    return None;
                }
                if remote.display_name.trim().is_empty() {
                    remote.display_name.clone_from(&remote.remote_id);
                }
                if remote.display_name.chars().count() > NAME_MAX_CHARS {
                    remote.display_name =
                        remote.display_name.chars().take(NAME_MAX_CHARS).collect();
                }
                let locks = Locks::new(provider.kind(), provider.preset(), &remote.remote_id);
                let capabilities =
                    Capabilities::defaults(provider.preset(), &remote).locked(&locks);
                Some(DetectedModel {
                    remote,
                    capabilities,
                })
            })
            .collect();
        let auto_enable = detected.len() <= MODEL_AUTO_ENABLE_MAX;
        self.models
            .sync(
                actor.user_id,
                id,
                detected,
                auto_enable,
                MAX_MODELS_PER_PROVIDER,
            )
            .await
    }

    /// The adapter's view of a connection owned by `actor`: the opened key and the network scope
    /// from `ALLOW_PRIVATE_NETWORK` and the owner's `is_admin`. Fails with `SECRET_UNREADABLE`
    /// when the stored key no longer opens.
    pub fn target(&self, actor: &Actor, provider: &Provider) -> DomainResult<ProviderTarget> {
        let api_key = provider
            .api_key_sealed()
            .map(|sealed| self.cipher.open(sealed, &provider.key_context()))
            .transpose()?;
        Ok(provider.target(api_key, self.network.scope(actor)))
    }

    async fn load(&self, actor: &Actor, id: ProviderId) -> DomainResult<Provider> {
        self.providers
            .get(actor.user_id, id)
            .await?
            .ok_or_else(|| not_found(id))
    }

    fn base_url(&self, actor: &Actor, text: &str) -> DomainResult<BaseUrl> {
        BaseUrl::parse(text, self.network.scope(actor)).map_err(|error| error.with_field("baseUrl"))
    }

    /// Seals a key for `provider`'s row; blank or absent means no key.
    fn seal(&self, provider: &Provider, key: Option<&str>) -> DomainResult<Option<Vec<u8>>> {
        key.map(str::trim)
            .filter(|key| !key.is_empty())
            .map(|key| self.cipher.seal(key.as_bytes(), &provider.key_context()))
            .transpose()
    }
}

/// The code of a model list Copilot does not accept.
const BAD_LIST: &str = "PROVIDER_BAD_RESPONSE";

fn not_found(id: ProviderId) -> DomainError {
    DomainError::not_found("Provider", id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::llm_gateway::{RemoteModel, ReportedCapabilities};
    use crate::shared::net::NetworkScope;
    use crate::testing::{
        FakeCipher, FixedClock, MemoryStore, ScriptedGateway, actor, admin, remote_model,
    };

    #[test]
    fn inputs_never_print_the_key() {
        let new = NewProvider {
            kind: ProviderKind::OpenAi,
            preset: Preset::OpenAi,
            name: "OpenAI".to_owned(),
            base_url: None,
            api_key: Some("sk-live-TOPSECRET".to_owned()),
        };
        let update = ProviderUpdate {
            api_key: Some(KeyChange::Set("sk-live-TOPSECRET".to_owned())),
            ..ProviderUpdate::default()
        };
        for printed in [format!("{new:?}"), format!("{update:?}")] {
            assert!(!printed.contains("TOPSECRET"), "{printed}");
            assert!(printed.contains("***"), "{printed}");
        }
        assert_eq!(format!("{:?}", KeyChange::Clear), "Clear");
    }

    struct Fixture {
        service: ProviderService,
        store: Arc<MemoryStore>,
        gateway: Arc<ScriptedGateway>,
        cipher: Arc<FakeCipher>,
    }

    fn fixture(network: PrivateNetwork) -> Fixture {
        let clock = Arc::new(FixedClock::default());
        let store = Arc::new(MemoryStore::new(clock.clone()));
        let gateway = Arc::new(ScriptedGateway::new());
        let cipher = Arc::new(FakeCipher::new());
        let service = ProviderService::new(
            store.clone(),
            store.clone(),
            store.clone(),
            gateway.clone(),
            cipher.clone(),
            clock,
            network,
        );
        Fixture {
            service,
            store,
            gateway,
            cipher,
        }
    }

    fn openai(key: Option<&str>) -> NewProvider {
        NewProvider {
            kind: ProviderKind::OpenAi,
            preset: Preset::OpenAi,
            name: "OpenAI".to_owned(),
            base_url: None,
            api_key: key.map(str::to_owned),
        }
    }

    fn ollama(url: &str) -> NewProvider {
        NewProvider {
            kind: ProviderKind::OpenAi,
            preset: Preset::Ollama,
            name: "Ollama".to_owned(),
            base_url: Some(url.to_owned()),
            api_key: None,
        }
    }

    fn validation_field(error: &DomainError) -> Option<&str> {
        match error {
            DomainError::Validation { field, .. } => field.as_deref(),
            _ => None,
        }
    }

    #[tokio::test]
    async fn creates_a_connection_with_the_preset_url_and_a_sealed_key() {
        let f = fixture(PrivateNetwork::Admins);
        let caller = actor();
        let provider = f
            .service
            .create(&caller, openai(Some("  sk-1  ")))
            .await
            .unwrap();
        assert_eq!(provider.base_url().as_str(), "https://api.openai.com/v1");
        assert_eq!(provider.user_id(), caller.user_id);
        let sealed = provider.api_key_sealed().expect("a sealed key");
        let opened = f.cipher.open(sealed, &provider.key_context()).unwrap();
        assert_eq!(opened.expose(), b"sk-1", "the key is trimmed and sealed");
        assert_eq!(f.service.list(&caller).await.unwrap(), [provider]);
    }

    #[tokio::test]
    async fn stores_no_key_when_it_is_blank() {
        let f = fixture(PrivateNetwork::Admins);
        let provider = f
            .service
            .create(&actor(), openai(Some("   ")))
            .await
            .unwrap();
        assert!(!provider.has_key());
    }

    #[tokio::test]
    async fn requires_a_base_url_for_custom_connections() {
        let f = fixture(PrivateNetwork::Admins);
        let input = NewProvider {
            kind: ProviderKind::OpenAi,
            preset: Preset::Custom,
            name: "Gateway".to_owned(),
            base_url: None,
            api_key: None,
        };
        let error = f.service.create(&actor(), input).await.unwrap_err();
        assert_eq!(error.to_string(), "Enter the base URL of this connection.");
        assert_eq!(validation_field(&error), Some("baseUrl"));
    }

    #[tokio::test]
    async fn validates_the_name_and_the_preset() {
        let f = fixture(PrivateNetwork::Admins);
        let mut input = openai(None);
        input.name = " ".to_owned();
        let error = f.service.create(&actor(), input).await.unwrap_err();
        assert_eq!(error.to_string(), "Provider name is required.");

        let mut input = openai(None);
        input.kind = ProviderKind::Anthropic;
        let error = f.service.create(&actor(), input).await.unwrap_err();
        assert_eq!(validation_field(&error), Some("preset"));
    }

    #[tokio::test]
    async fn allows_private_hosts_only_for_admins_by_default() {
        let f = fixture(PrivateNetwork::Admins);
        let error = f
            .service
            .create(&actor(), ollama("http://localhost:11434/v1"))
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "This account cannot connect to private network addresses."
        );
        assert_eq!(validation_field(&error), Some("baseUrl"));
        f.service
            .create(&admin(), ollama("http://localhost:11434/v1"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn follows_the_private_network_setting() {
        let none = fixture(PrivateNetwork::None);
        assert!(
            none.service
                .create(&admin(), ollama("http://10.0.0.2:11434/v1"))
                .await
                .is_err()
        );
        let all = fixture(PrivateNetwork::All);
        assert!(
            all.service
                .create(&actor(), ollama("http://10.0.0.2:11434/v1"))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn hides_other_users_connections() {
        let f = fixture(PrivateNetwork::Admins);
        let alice = actor();
        let bob = actor();
        let provider = f
            .service
            .create(&alice, openai(Some("sk-1")))
            .await
            .unwrap();
        assert!(f.service.list(&bob).await.unwrap().is_empty());
        for error in [
            f.service.get(&bob, provider.id()).await.unwrap_err(),
            f.service
                .update(&bob, provider.id(), ProviderUpdate::default())
                .await
                .unwrap_err(),
            f.service.delete(&bob, provider.id()).await.unwrap_err(),
            f.service.refresh(&bob, provider.id()).await.unwrap_err(),
        ] {
            assert!(
                matches!(error, DomainError::EntityNotFound { .. }),
                "{error}"
            );
        }
        assert!(f.service.get(&alice, provider.id()).await.is_ok());
    }

    #[tokio::test]
    async fn clears_the_key_when_the_origin_changes_without_a_new_key() {
        let f = fixture(PrivateNetwork::Admins);
        let caller = actor();
        let provider = f
            .service
            .create(&caller, openai(Some("sk-1")))
            .await
            .unwrap();

        let same_origin = ProviderUpdate {
            base_url: Some("https://api.openai.com/v2".to_owned()),
            ..ProviderUpdate::default()
        };
        let updated = f
            .service
            .update(&caller, provider.id(), same_origin)
            .await
            .unwrap();
        assert!(updated.has_key(), "a path change keeps the key");

        let new_host = ProviderUpdate {
            base_url: Some("https://gateway.example.com/v1".to_owned()),
            ..ProviderUpdate::default()
        };
        let updated = f
            .service
            .update(&caller, provider.id(), new_host)
            .await
            .unwrap();
        assert!(!updated.has_key(), "a new origin clears the key");
        assert_eq!(
            updated.base_url().as_str(),
            "https://gateway.example.com/v1"
        );
    }

    #[tokio::test]
    async fn keeps_a_new_key_given_with_a_new_origin() {
        let f = fixture(PrivateNetwork::Admins);
        let caller = actor();
        let provider = f
            .service
            .create(&caller, openai(Some("sk-1")))
            .await
            .unwrap();
        let changes = ProviderUpdate {
            base_url: Some("https://gateway.example.com/v1".to_owned()),
            api_key: Some(KeyChange::Set("sk-2".to_owned())),
            ..ProviderUpdate::default()
        };
        let updated = f
            .service
            .update(&caller, provider.id(), changes)
            .await
            .unwrap();
        let opened = f
            .cipher
            .open(updated.api_key_sealed().unwrap(), &updated.key_context())
            .unwrap();
        assert_eq!(opened.expose(), b"sk-2");
    }

    #[tokio::test]
    async fn renames_disables_and_clears_the_key() {
        let f = fixture(PrivateNetwork::Admins);
        let caller = actor();
        let provider = f
            .service
            .create(&caller, openai(Some("sk-1")))
            .await
            .unwrap();
        let changes = ProviderUpdate {
            name: Some("Work".to_owned()),
            enabled: Some(false),
            api_key: Some(KeyChange::Clear),
            ..ProviderUpdate::default()
        };
        let updated = f
            .service
            .update(&caller, provider.id(), changes)
            .await
            .unwrap();
        assert_eq!(updated.name(), "Work");
        assert!(!updated.enabled());
        assert!(!updated.has_key());
        assert_eq!(f.service.list(&caller).await.unwrap(), [updated]);
    }

    #[tokio::test]
    async fn reports_the_key_state() {
        let f = fixture(PrivateNetwork::Admins);
        let caller = actor();
        let without = f.service.create(&caller, openai(None)).await.unwrap();
        let with = f
            .service
            .create(&caller, openai(Some("sk-1")))
            .await
            .unwrap();
        assert_eq!(
            f.service.get(&caller, without.id()).await.unwrap().key,
            KeyState::Missing
        );
        assert_eq!(
            f.service.get(&caller, with.id()).await.unwrap().key,
            KeyState::Set
        );
        f.cipher.rotate();
        assert_eq!(
            f.service.get(&caller, with.id()).await.unwrap().key,
            KeyState::Unreadable
        );
    }

    #[tokio::test]
    async fn builds_the_target_with_the_opened_key_and_the_owner_scope() {
        let f = fixture(PrivateNetwork::Admins);
        let caller = admin();
        let provider = f
            .service
            .create(&caller, openai(Some("sk-1")))
            .await
            .unwrap();
        f.gateway.push_models(vec![]);
        f.service.refresh(&caller, provider.id()).await.unwrap();
        let target = f.gateway.targets().pop().expect("the gateway was called");
        assert_eq!(target.provider_id, provider.id());
        assert_eq!(
            target.api_key.as_ref().and_then(|key| key.text()),
            Some("sk-1")
        );
        assert_eq!(target.network, NetworkScope::Private);
        assert_eq!(target.base_url.as_str(), "https://api.openai.com/v1");
    }

    #[tokio::test]
    async fn refuses_to_refresh_with_an_unreadable_key() {
        let f = fixture(PrivateNetwork::Admins);
        let caller = actor();
        let provider = f
            .service
            .create(&caller, openai(Some("sk-1")))
            .await
            .unwrap();
        f.cipher.rotate();
        let error = f.service.refresh(&caller, provider.id()).await.unwrap_err();
        assert_eq!(error.code(), crate::ports::secret_cipher::SECRET_UNREADABLE);
        assert!(
            f.gateway.targets().is_empty(),
            "the provider is never called"
        );
    }

    #[tokio::test]
    async fn passes_gateway_errors_through() {
        let f = fixture(PrivateNetwork::Admins);
        let caller = actor();
        let provider = f
            .service
            .create(&caller, openai(Some("sk-1")))
            .await
            .unwrap();
        f.gateway.push_error(DomainError::business_rule(
            "Key rejected: open provider settings.",
            "PROVIDER_AUTH",
        ));
        let error = f.service.refresh(&caller, provider.id()).await.unwrap_err();
        assert_eq!(error.code(), "PROVIDER_AUTH");
    }

    #[tokio::test]
    async fn refresh_applies_defaults_and_locks_and_auto_enables_small_lists() {
        let f = fixture(PrivateNetwork::Admins);
        let caller = actor();
        let provider = f
            .service
            .create(&caller, openai(Some("sk-1")))
            .await
            .unwrap();
        f.gateway.push_models(vec![
            remote_model("gpt-5.6-terra"),
            remote_model("gpt-6.1-sol"),
            remote_model(" gpt-5.6-terra "),
            remote_model(""),
        ]);
        let report = f.service.refresh(&caller, provider.id()).await.unwrap();
        assert_eq!(
            report,
            SyncReport {
                added: 2,
                updated: 0,
                unavailable: 0,
                enabled: 2
            }
        );
        let models = ModelRepository::list(
            f.store.as_ref(),
            caller.user_id,
            ModelQuery::provider(provider.id()),
        )
        .await
        .unwrap();
        let terra = models
            .iter()
            .find(|model| model.remote_id() == "gpt-5.6-terra")
            .unwrap();
        let sol = models
            .iter()
            .find(|model| model.remote_id() == "gpt-6.1-sol")
            .unwrap();
        assert!(terra.capabilities().pdf, "openai preset turns PDF on");
        assert!(terra.capabilities().tools);
        assert!(!sol.capabilities().tools, "gpt-6 tools are locked off");
        assert!(terra.enabled() && sol.enabled());
    }

    #[tokio::test]
    async fn refresh_leaves_large_lists_for_the_user_to_pick() {
        let f = fixture(PrivateNetwork::Admins);
        let caller = actor();
        let provider = f
            .service
            .create(&caller, openai(Some("sk-1")))
            .await
            .unwrap();
        let listed: Vec<RemoteModel> = (0..=MODEL_AUTO_ENABLE_MAX)
            .map(|index| remote_model(&format!("model-{index}")))
            .collect();
        f.gateway.push_models(listed);
        let report = f.service.refresh(&caller, provider.id()).await.unwrap();
        assert_eq!(report.added, 21);
        assert_eq!(report.enabled, 0);
    }

    #[tokio::test]
    async fn refresh_refuses_lists_a_connection_cannot_keep() {
        let f = fixture(PrivateNetwork::Admins);
        let caller = actor();
        let provider = f
            .service
            .create(&caller, openai(Some("sk-1")))
            .await
            .unwrap();
        let stored = || async {
            ModelRepository::list(
                f.store.as_ref(),
                caller.user_id,
                ModelQuery::provider(provider.id()),
            )
            .await
            .unwrap()
        };

        let too_many: Vec<RemoteModel> = (0..=MAX_MODELS_PER_PROVIDER)
            .map(|index| remote_model(&format!("m{index}")))
            .collect();
        f.gateway.push_models(too_many);
        let error = f.service.refresh(&caller, provider.id()).await.unwrap_err();
        assert_eq!(error.code(), BAD_LIST);
        assert_eq!(
            error.to_string(),
            "The provider listed more than 2000 models, more than a connection can keep."
        );
        assert!(stored().await.is_empty(), "nothing is stored");

        let long_id = "x".repeat(MODEL_ID_MAX_CHARS + 1);
        f.gateway
            .push_models(vec![remote_model("ok"), remote_model(&long_id)]);
        let error = f.service.refresh(&caller, provider.id()).await.unwrap_err();
        assert_eq!(error.code(), BAD_LIST);
        assert!(stored().await.is_empty());

        let mut wordy = remote_model(&"y".repeat(MODEL_ID_MAX_CHARS));
        wordy.display_name = "é".repeat(NAME_MAX_CHARS + 50);
        f.gateway.push_models(vec![wordy]);
        f.service.refresh(&caller, provider.id()).await.unwrap();
        let models = stored().await;
        assert_eq!(models[0].display_name(), "é".repeat(NAME_MAX_CHARS));
    }

    #[tokio::test]
    async fn refresh_marks_missing_models_unavailable_and_keeps_user_settings() {
        let f = fixture(PrivateNetwork::Admins);
        let caller = actor();
        let provider = f
            .service
            .create(&caller, openai(Some("sk-1")))
            .await
            .unwrap();
        f.gateway
            .push_models(vec![remote_model("a"), remote_model("b")]);
        f.service.refresh(&caller, provider.id()).await.unwrap();

        let mut vision = remote_model("a");
        vision.reported = ReportedCapabilities {
            vision: Some(true),
            ..ReportedCapabilities::default()
        };
        vision.display_name = "Renamed upstream".to_owned();
        f.gateway.push_models(vec![vision, remote_model("c")]);
        let report = f.service.refresh(&caller, provider.id()).await.unwrap();
        assert_eq!(
            report,
            SyncReport {
                added: 1,
                updated: 1,
                unavailable: 1,
                enabled: 1
            }
        );
        let models = ModelRepository::list(
            f.store.as_ref(),
            caller.user_id,
            ModelQuery::provider(provider.id()),
        )
        .await
        .unwrap();
        let by_id = |id: &str| models.iter().find(|model| model.remote_id() == id).unwrap();
        assert!(by_id("a").available());
        assert!(
            by_id("a").capabilities().vision,
            "detected capabilities are replaced"
        );
        assert_eq!(
            by_id("a").display_name(),
            "Model a",
            "the stored name stays"
        );
        assert!(!by_id("b").available(), "never deleted, only unavailable");
        assert!(by_id("c").available());
    }

    #[tokio::test]
    async fn deletes_a_connection_with_its_models() {
        let f = fixture(PrivateNetwork::Admins);
        let caller = actor();
        let provider = f
            .service
            .create(&caller, openai(Some("sk-1")))
            .await
            .unwrap();
        f.gateway.push_models(vec![remote_model("a")]);
        f.service.refresh(&caller, provider.id()).await.unwrap();
        f.service.delete(&caller, provider.id()).await.unwrap();
        assert!(f.service.list(&caller).await.unwrap().is_empty());
        let models = ModelRepository::list(f.store.as_ref(), caller.user_id, ModelQuery::default())
            .await
            .unwrap();
        assert!(models.is_empty());
        let error = f.service.delete(&caller, provider.id()).await.unwrap_err();
        assert!(matches!(error, DomainError::EntityNotFound { .. }));
    }
}
