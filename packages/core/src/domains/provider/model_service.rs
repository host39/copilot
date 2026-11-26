//! Model picker and model settings use cases.

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::future::try_join;

use crate::domains::provider::capabilities::{
    Capabilities, CapabilityOverrides, Locks, reasoning_off_with_tools,
};
use crate::domains::provider::entities::{Model, Provider};
use crate::domains::provider::ports::{ModelQuery, ModelRepository, ProviderRepository};
use crate::domains::provider::values::{Preset, ProviderKind};
use crate::domains::schedule::{MODEL_TURNED_OFF, ScheduleRepository};
use crate::ports::clock::Clock;
use crate::shared::actor::Actor;
use crate::shared::constants::NAME_MAX_CHARS;
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{ModelId, ProviderId};
use crate::shared::text;

/// Largest token limit a model may store (the columns are Postgres `integer`).
const TOKEN_LIMIT_MAX: u32 = i32::MAX as u32;

/// A model with what the picker and the model settings show.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelView {
    pub model: Model,
    pub provider_name: String,
    pub kind: ProviderKind,
    pub preset: Preset,
    /// After overrides and locks.
    pub capabilities: Capabilities,
    /// Flags the UI shows without a toggle.
    pub locks: Locks,
    /// The `openai` preset sends `reasoning_effort: none` whenever tools are on.
    pub reasoning_off_with_tools: bool,
}

impl ModelView {
    fn new(model: Model, provider: &Provider) -> Self {
        Self {
            capabilities: model.effective(provider),
            locks: model.locks(provider),
            reasoning_off_with_tools: reasoning_off_with_tools(
                provider.preset(),
                model.remote_id(),
            ),
            provider_name: provider.name().to_owned(),
            kind: provider.kind(),
            preset: provider.preset(),
            model,
        }
    }
}

/// Input of [`ModelService::update`]; `None` keeps a field. The inner `Option` of the token
/// limits clears a value.
#[derive(Clone, Debug, Default)]
pub struct ModelUpdate {
    pub enabled: Option<bool>,
    pub display_name: Option<String>,
    /// Replaces every override.
    pub capability_overrides: Option<CapabilityOverrides>,
    pub context_window: Option<Option<u32>>,
    pub max_output_tokens: Option<Option<u32>>,
}

#[derive(Clone)]
pub struct ModelService {
    models: Arc<dyn ModelRepository>,
    providers: Arc<dyn ProviderRepository>,
    schedules: Arc<dyn ScheduleRepository>,
    clock: Arc<dyn Clock>,
}

impl ModelService {
    pub fn new(
        models: Arc<dyn ModelRepository>,
        providers: Arc<dyn ProviderRepository>,
        schedules: Arc<dyn ScheduleRepository>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            models,
            providers,
            schedules,
            clock,
        }
    }

    /// The picker (`ModelQuery::picker()`) or one connection's models, grouped by connection
    /// name and sorted by display name.
    pub async fn list(&self, actor: &Actor, query: ModelQuery) -> DomainResult<Vec<ModelView>> {
        if let Some(id) = query.provider {
            self.providers
                .get(actor.user_id, id)
                .await?
                .ok_or_else(|| DomainError::not_found("Provider", id.to_string()))?;
        }
        let (models, providers) = try_join(
            self.models.list(actor.user_id, query),
            self.providers.list(actor.user_id),
        )
        .await?;
        let providers: HashMap<ProviderId, Provider> = providers
            .into_iter()
            .map(|provider| (provider.id(), provider))
            .collect();
        let mut views: Vec<ModelView> = models
            .into_iter()
            .filter_map(|model| {
                let provider = providers.get(&model.provider_id())?;
                Some(ModelView::new(model, provider))
            })
            .collect();
        views.sort_by(|left, right| {
            (left.provider_name.as_str(), left.model.display_name())
                .cmp(&(right.provider_name.as_str(), right.model.display_name()))
        });
        Ok(views)
    }

    pub async fn get(&self, actor: &Actor, id: ModelId) -> DomainResult<ModelView> {
        let (model, provider) = self.load(actor, id).await?;
        Ok(ModelView::new(model, &provider))
    }

    /// Enables, renames, overrides capabilities, or sets token limits. Overriding a locked flag
    /// is rejected. Turning a model off pauses the schedules that run it ("Model turned off").
    pub async fn update(
        &self,
        actor: &Actor,
        id: ModelId,
        changes: ModelUpdate,
    ) -> DomainResult<ModelView> {
        let (mut model, provider) = self.load(actor, id).await?;
        let turned_off = changes.enabled == Some(false) && model.enabled();
        if let Some(enabled) = changes.enabled {
            model = model.with_enabled(enabled);
        }
        if let Some(name) = &changes.display_name {
            let name = text::name(name, "Model name", "displayName", NAME_MAX_CHARS)?;
            model = model.with_display_name(name);
        }
        if let Some(overrides) = changes.capability_overrides {
            let locks = model.locks(&provider);
            if let Some(lock) = overrides.flags().find_map(|flag| locks.get(flag)) {
                return Err(DomainError::validation(format!(
                    "{} cannot be changed for this model. {}",
                    lock.capability.label(),
                    lock.reason
                ))
                .with_field("capabilityOverrides")
                .with_context("capability", lock.capability.as_str()));
            }
            model = model.with_overrides(overrides);
        }
        let context_window = changes.context_window.unwrap_or(model.context_window());
        let max_output_tokens = changes
            .max_output_tokens
            .unwrap_or(model.max_output_tokens());
        limit("The context window", "contextWindow", context_window)?;
        limit("Max output tokens", "maxOutputTokens", max_output_tokens)?;
        if let (Some(window), Some(output)) = (context_window, max_output_tokens)
            && output >= window
        {
            return Err(DomainError::validation(
                "Max output tokens must be less than the context window.",
            )
            .with_field("maxOutputTokens"));
        }
        let model = model
            .with_limits(context_window, max_output_tokens)
            .touched(self.clock.now());
        self.models.update(&model).await?;
        if turned_off {
            self.schedules
                .disable_for_model(actor.user_id, id, MODEL_TURNED_OFF)
                .await?;
        }
        Ok(ModelView::new(model, &provider))
    }

    async fn load(&self, actor: &Actor, id: ModelId) -> DomainResult<(Model, Provider)> {
        let not_found = || DomainError::not_found("Model", id.to_string());
        let model = self
            .models
            .get(actor.user_id, id)
            .await?
            .ok_or_else(not_found)?;
        let provider = self
            .providers
            .get(actor.user_id, model.provider_id())
            .await?
            .ok_or_else(not_found)?;
        Ok((model, provider))
    }
}

fn limit(label: &str, field: &str, value: Option<u32>) -> DomainResult<()> {
    match value {
        Some(0) => Err(
            DomainError::validation(format!("{label} must be greater than zero."))
                .with_field(field),
        ),
        Some(value) if value > TOKEN_LIMIT_MAX => Err(DomainError::validation(format!(
            "{label} must be at most {TOKEN_LIMIT_MAX}."
        ))
        .with_field(field)),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::provider::capabilities::Capability;
    use crate::domains::provider::service::{NewProvider, ProviderService, ProviderUpdate};
    use crate::ports::llm_gateway::RemoteModel;
    use crate::shared::net::PrivateNetwork;
    use crate::shared::reasoning::ReasoningLevel;
    use crate::testing::{
        FakeCipher, FixedClock, MemoryStore, ScriptedGateway, actor, remote_model,
    };

    struct Fixture {
        models: ModelService,
        providers: ProviderService,
        gateway: Arc<ScriptedGateway>,
    }

    fn fixture() -> Fixture {
        let clock = Arc::new(FixedClock::default());
        let store = Arc::new(MemoryStore::new(clock.clone()));
        let gateway = Arc::new(ScriptedGateway::new());
        let providers = ProviderService::new(
            store.clone(),
            store.clone(),
            store.clone(),
            gateway.clone(),
            Arc::new(FakeCipher::new()),
            clock.clone(),
            PrivateNetwork::Admins,
        );
        Fixture {
            models: ModelService::new(store.clone(), store.clone(), store, clock),
            providers,
            gateway,
        }
    }

    async fn connect(
        f: &Fixture,
        caller: &Actor,
        kind: ProviderKind,
        preset: Preset,
        name: &str,
        listed: Vec<RemoteModel>,
    ) -> Provider {
        let provider = f
            .providers
            .create(
                caller,
                NewProvider {
                    kind,
                    preset,
                    name: name.to_owned(),
                    base_url: match preset {
                        Preset::Custom | Preset::Ollama => {
                            Some("https://gateway.example.com/v1".to_owned())
                        }
                        _ => None,
                    },
                    api_key: Some("key".to_owned()),
                },
            )
            .await
            .unwrap();
        f.gateway.push_models(listed);
        f.providers.refresh(caller, provider.id()).await.unwrap();
        provider
    }

    async fn model(f: &Fixture, caller: &Actor, provider: &Provider, remote_id: &str) -> ModelView {
        f.models
            .list(caller, ModelQuery::provider(provider.id()))
            .await
            .unwrap()
            .into_iter()
            .find(|view| view.model.remote_id() == remote_id)
            .unwrap()
    }

    #[tokio::test]
    async fn the_picker_lists_usable_models_of_enabled_connections_by_connection() {
        let f = fixture();
        let caller = actor();
        let openai = connect(
            &f,
            &caller,
            ProviderKind::OpenAi,
            Preset::OpenAi,
            "OpenAI",
            vec![remote_model("b"), remote_model("a")],
        )
        .await;
        let claude = connect(
            &f,
            &caller,
            ProviderKind::Anthropic,
            Preset::Anthropic,
            "Anthropic",
            vec![remote_model("c")],
        )
        .await;

        let off = model(&f, &caller, &openai, "b").await;
        let changes = ModelUpdate {
            enabled: Some(false),
            ..ModelUpdate::default()
        };
        f.models
            .update(&caller, off.model.id(), changes)
            .await
            .unwrap();

        let picker = f.models.list(&caller, ModelQuery::picker()).await.unwrap();
        let names: Vec<_> = picker
            .iter()
            .map(|view| (view.provider_name.as_str(), view.model.remote_id()))
            .collect();
        assert_eq!(names, [("Anthropic", "c"), ("OpenAI", "a")]);

        let disable = ProviderUpdate {
            enabled: Some(false),
            ..ProviderUpdate::default()
        };
        f.providers
            .update(&caller, claude.id(), disable)
            .await
            .unwrap();
        let picker = f.models.list(&caller, ModelQuery::picker()).await.unwrap();
        assert_eq!(
            picker.len(),
            1,
            "a disabled connection's models leave the picker"
        );

        let all = f
            .models
            .list(&caller, ModelQuery::provider(openai.id()))
            .await
            .unwrap();
        assert_eq!(all.len(), 2, "the connection page lists every known model");
    }

    #[tokio::test]
    async fn the_picker_hides_unavailable_models() {
        let f = fixture();
        let caller = actor();
        let provider = connect(
            &f,
            &caller,
            ProviderKind::OpenAi,
            Preset::Ollama,
            "Local",
            vec![remote_model("a"), remote_model("b")],
        )
        .await;
        f.gateway.push_models(vec![remote_model("a")]);
        f.providers.refresh(&caller, provider.id()).await.unwrap();
        let picker = f.models.list(&caller, ModelQuery::picker()).await.unwrap();
        assert_eq!(picker.len(), 1);
        assert_eq!(picker[0].model.remote_id(), "a");
    }

    #[tokio::test]
    async fn shows_locks_effective_capabilities_and_the_reasoning_note() {
        let f = fixture();
        let caller = actor();
        let openai = connect(
            &f,
            &caller,
            ProviderKind::OpenAi,
            Preset::OpenAi,
            "OpenAI",
            vec![remote_model("gpt-5.6-terra"), remote_model("gpt-6.1-sol")],
        )
        .await;
        let terra = model(&f, &caller, &openai, "gpt-5.6-terra").await;
        assert!(terra.reasoning_off_with_tools);
        assert!(terra.capabilities.tools);
        let sol = model(&f, &caller, &openai, "gpt-6.1-sol").await;
        assert!(sol.locks.contains(Capability::Tools));
        assert!(!sol.capabilities.tools);
    }

    #[tokio::test]
    async fn rejects_overrides_of_locked_flags() {
        let f = fixture();
        let caller = actor();
        let claude = connect(
            &f,
            &caller,
            ProviderKind::Anthropic,
            Preset::Anthropic,
            "Anthropic",
            vec![remote_model("claude-opus-5-5")],
        )
        .await;
        let view = model(&f, &caller, &claude, "claude-opus-5-5").await;
        let changes = ModelUpdate {
            capability_overrides: Some(CapabilityOverrides {
                audio_in: Some(true),
                ..CapabilityOverrides::default()
            }),
            ..ModelUpdate::default()
        };
        let error = f
            .models
            .update(&caller, view.model.id(), changes)
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Audio input cannot be changed for this model. Claude models do not take or return audio."
        );
    }

    #[tokio::test]
    async fn applies_overrides_names_and_limits() {
        let f = fixture();
        let caller = actor();
        let provider = connect(
            &f,
            &caller,
            ProviderKind::OpenAi,
            Preset::Custom,
            "Gateway",
            vec![remote_model("m")],
        )
        .await;
        let view = model(&f, &caller, &provider, "m").await;
        let changes = ModelUpdate {
            display_name: Some("  My model ".to_owned()),
            capability_overrides: Some(CapabilityOverrides {
                vision: Some(true),
                reasoning: Some(true),
                ..CapabilityOverrides::default()
            }),
            context_window: Some(Some(32_000)),
            max_output_tokens: Some(Some(4_000)),
            ..ModelUpdate::default()
        };
        let updated = f
            .models
            .update(&caller, view.model.id(), changes)
            .await
            .unwrap();
        assert_eq!(updated.model.display_name(), "My model");
        assert!(updated.capabilities.vision);
        assert_eq!(updated.capabilities.reasoning_levels, ReasoningLevel::ALL);
        assert!(
            !updated.model.capabilities().vision,
            "detected values stay as reported"
        );
        assert_eq!(updated.model.context_window(), Some(32_000));
        assert_eq!(
            f.models.get(&caller, view.model.id()).await.unwrap(),
            updated
        );

        let clear = ModelUpdate {
            max_output_tokens: Some(None),
            ..ModelUpdate::default()
        };
        let cleared = f
            .models
            .update(&caller, view.model.id(), clear)
            .await
            .unwrap();
        assert_eq!(cleared.model.max_output_tokens(), None);
        assert_eq!(cleared.model.context_window(), Some(32_000));
    }

    #[tokio::test]
    async fn validates_token_limits() {
        let f = fixture();
        let caller = actor();
        let provider = connect(
            &f,
            &caller,
            ProviderKind::OpenAi,
            Preset::Custom,
            "Gateway",
            vec![remote_model("m")],
        )
        .await;
        let id = model(&f, &caller, &provider, "m").await.model.id();
        let cases = [
            (
                Some(Some(0)),
                None,
                "The context window must be greater than zero.",
            ),
            (
                None,
                Some(Some(0)),
                "Max output tokens must be greater than zero.",
            ),
            (
                Some(Some(u32::MAX)),
                None,
                "The context window must be at most 2147483647.",
            ),
            (
                Some(Some(8_000)),
                Some(Some(8_000)),
                "Max output tokens must be less than the context window.",
            ),
        ];
        for (context_window, max_output_tokens, message) in cases {
            let changes = ModelUpdate {
                context_window,
                max_output_tokens,
                ..ModelUpdate::default()
            };
            let error = f.models.update(&caller, id, changes).await.unwrap_err();
            assert_eq!(error.to_string(), message);
        }
    }

    #[tokio::test]
    async fn validates_the_display_name() {
        let f = fixture();
        let caller = actor();
        let provider = connect(
            &f,
            &caller,
            ProviderKind::OpenAi,
            Preset::Custom,
            "Gateway",
            vec![remote_model("m")],
        )
        .await;
        let id = model(&f, &caller, &provider, "m").await.model.id();
        let changes = ModelUpdate {
            display_name: Some(String::new()),
            ..ModelUpdate::default()
        };
        let error = f.models.update(&caller, id, changes).await.unwrap_err();
        assert_eq!(error.to_string(), "Model name is required.");
    }

    #[tokio::test]
    async fn hides_other_users_models() {
        let f = fixture();
        let alice = actor();
        let bob = actor();
        let provider = connect(
            &f,
            &alice,
            ProviderKind::OpenAi,
            Preset::Custom,
            "Gateway",
            vec![remote_model("m")],
        )
        .await;
        let id = model(&f, &alice, &provider, "m").await.model.id();
        assert!(
            f.models
                .list(&bob, ModelQuery::picker())
                .await
                .unwrap()
                .is_empty()
        );
        let error = f
            .models
            .list(&bob, ModelQuery::provider(provider.id()))
            .await
            .unwrap_err();
        assert!(matches!(error, DomainError::EntityNotFound { .. }));
        let error = f.models.get(&bob, id).await.unwrap_err();
        assert!(matches!(error, DomainError::EntityNotFound { .. }));
        let error = f
            .models
            .update(&bob, id, ModelUpdate::default())
            .await
            .unwrap_err();
        assert!(matches!(error, DomainError::EntityNotFound { .. }));
    }
}
