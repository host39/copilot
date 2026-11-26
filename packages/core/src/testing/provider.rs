//! In-memory `ProviderRepository` and `ModelRepository`.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domains::provider::{
    CapabilityOverrides, DetectedModel, Model, ModelParts, ModelQuery, ModelRepository, Provider,
    ProviderRepository, ProviderUsage, SyncReport,
};
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{ModelId, ProviderId, UserId};
use crate::testing::store::{MemoryStore, Tables, duplicate};

#[async_trait]
impl ProviderRepository for MemoryStore {
    async fn list(&self, user: UserId) -> DomainResult<Vec<Provider>> {
        let tables = self.tables();
        Ok(tables
            .providers
            .values()
            .filter(|provider| provider.user_id() == user)
            .cloned()
            .collect())
    }

    async fn get(&self, user: UserId, id: ProviderId) -> DomainResult<Option<Provider>> {
        let tables = self.tables();
        Ok(tables
            .providers
            .get(&id)
            .filter(|provider| provider.user_id() == user)
            .cloned())
    }

    async fn insert(&self, provider: &Provider) -> DomainResult<()> {
        let mut tables = self.tables();
        if tables.providers.contains_key(&provider.id()) {
            return Err(duplicate("providers"));
        }
        tables.providers.insert(provider.id(), provider.clone());
        Ok(())
    }

    async fn update(&self, provider: &Provider) -> DomainResult<()> {
        let mut tables = self.tables();
        if let Some(stored) = tables.providers.get_mut(&provider.id())
            && stored.user_id() == provider.user_id()
        {
            *stored = provider.clone();
        }
        Ok(())
    }

    async fn delete(&self, user: UserId, id: ProviderId) -> DomainResult<bool> {
        let mut tables = self.tables();
        if !tables
            .providers
            .get(&id)
            .is_some_and(|provider| provider.user_id() == user)
        {
            return Ok(false);
        }
        tables.providers.remove(&id);
        let removed: Vec<ModelId> = tables
            .models
            .values()
            .filter(|model| model.provider_id() == id)
            .map(Model::id)
            .collect();
        for model in removed {
            tables.delete_model(model);
        }
        tables.clear_provider_references(id);
        Ok(true)
    }

    async fn usage(&self, user: UserId, id: ProviderId) -> DomainResult<ProviderUsage> {
        let tables = self.tables();
        Ok(tables.provider_usage(user, id))
    }
}

#[async_trait]
impl ModelRepository for MemoryStore {
    async fn list(&self, user: UserId, query: ModelQuery) -> DomainResult<Vec<Model>> {
        let tables = self.tables();
        let mut models: Vec<Model> = tables
            .models
            .values()
            .filter(|model| model.user_id() == user)
            .filter(|model| query.provider.is_none_or(|id| model.provider_id() == id))
            .filter(|model| {
                !query.usable
                    || (model.is_usable()
                        && tables
                            .providers
                            .get(&model.provider_id())
                            .is_some_and(Provider::enabled))
            })
            .cloned()
            .collect();
        models.sort_by(|left, right| {
            (left.provider_id(), left.display_name())
                .cmp(&(right.provider_id(), right.display_name()))
        });
        Ok(models)
    }

    async fn get(&self, user: UserId, id: ModelId) -> DomainResult<Option<Model>> {
        let tables = self.tables();
        Ok(tables
            .models
            .get(&id)
            .filter(|model| model.user_id() == user)
            .cloned())
    }

    async fn sync(
        &self,
        user: UserId,
        provider: ProviderId,
        remote: Vec<DetectedModel>,
        auto_enable: bool,
        keep: usize,
    ) -> DomainResult<SyncReport> {
        let now = self.now();
        let mut tables = self.tables();
        if !tables
            .providers
            .get(&provider)
            .is_some_and(|stored| stored.user_id() == user)
        {
            return Err(DomainError::Internal {
                key: "testing.models.sync.provider-missing".to_owned(),
                source: None,
            });
        }
        let known: HashMap<String, ModelId> = tables
            .models
            .values()
            .filter(|model| model.provider_id() == provider)
            .map(|model| (model.remote_id().to_owned(), model.id()))
            .collect();
        let mut report = SyncReport::default();
        let mut listed = Vec::new();
        for detected in remote {
            let remote = detected.remote;
            if let Some(id) = known.get(&remote.remote_id) {
                let stored = tables.models.get(id).expect("a known model").clone();
                let parts = ModelParts {
                    available: true,
                    capabilities: detected.capabilities,
                    context_window: remote.context_window.or(stored.context_window()),
                    max_output_tokens: remote.max_output_tokens.or(stored.max_output_tokens()),
                    updated_at: now,
                    ..parts(&stored)
                };
                tables.models.insert(*id, Model::restore(parts));
                listed.push(*id);
                report.updated += 1;
            } else {
                let model = Model::restore(ModelParts {
                    id: ModelId::generate(),
                    user_id: user,
                    provider_id: provider,
                    remote_id: remote.remote_id,
                    display_name: remote.display_name,
                    enabled: auto_enable,
                    available: true,
                    context_window: remote.context_window,
                    max_output_tokens: remote.max_output_tokens,
                    capabilities: detected.capabilities,
                    overrides: CapabilityOverrides::default(),
                    created_at: now,
                    updated_at: now,
                });
                listed.push(model.id());
                tables.models.insert(model.id(), model);
                report.added += 1;
                if auto_enable {
                    report.enabled += 1;
                }
            }
        }
        for id in known.values() {
            if listed.contains(id) {
                continue;
            }
            let stored = tables.models.get(id).expect("a known model").clone();
            if stored.available() {
                let parts = ModelParts {
                    available: false,
                    updated_at: now,
                    ..parts(&stored)
                };
                tables.models.insert(*id, Model::restore(parts));
                report.unavailable += 1;
            }
        }
        let held = known.len() + report.added as usize;
        if held > keep {
            let used = |id: ModelId| {
                tables
                    .chats
                    .values()
                    .any(|chat| chat.model_id() == Some(id))
                    || tables
                        .messages
                        .values()
                        .any(|message| message.model_id() == Some(id))
                    || tables
                        .projects
                        .values()
                        .any(|project| project.default_model_id() == Some(id))
                    || tables
                        .schedules
                        .values()
                        .any(|schedule| schedule.model_id() == Some(id))
            };
            let mut spare: Vec<(DateTime<Utc>, ModelId)> = tables
                .models
                .values()
                .filter(|model| model.provider_id() == provider)
                .filter(|model| !model.available() && !model.enabled() && !used(model.id()))
                .map(|model| (model.updated_at(), model.id()))
                .collect();
            spare.sort();
            for (_, id) in spare.into_iter().take(held - keep) {
                tables.models.remove(&id);
            }
        }
        Ok(report)
    }

    async fn update(&self, model: &Model) -> DomainResult<()> {
        let mut tables = self.tables();
        if let Some(stored) = tables.models.get_mut(&model.id())
            && stored.user_id() == model.user_id()
        {
            *stored = model.clone();
        }
        Ok(())
    }
}

impl Tables {
    /// Deletes a model and nulls every reference to it (`ON DELETE SET NULL`).
    pub(crate) fn delete_model(&mut self, id: ModelId) {
        self.models.remove(&id);
        self.clear_model_references(id);
    }

    pub(crate) fn provider_usage(&self, user: UserId, id: ProviderId) -> ProviderUsage {
        let models: Vec<ModelId> = self
            .models
            .values()
            .filter(|model| model.user_id() == user && model.provider_id() == id)
            .map(Model::id)
            .collect();
        self.model_usage(user, &models)
    }
}

fn parts(model: &Model) -> ModelParts {
    ModelParts {
        id: model.id(),
        user_id: model.user_id(),
        provider_id: model.provider_id(),
        remote_id: model.remote_id().to_owned(),
        display_name: model.display_name().to_owned(),
        enabled: model.enabled(),
        available: model.available(),
        context_window: model.context_window(),
        max_output_tokens: model.max_output_tokens(),
        capabilities: model.capabilities().clone(),
        overrides: model.overrides().clone(),
        created_at: model.created_at(),
        updated_at: model.updated_at(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::domains::provider::{Capabilities, Preset, ProviderKind};
    use crate::ports::clock::Clock;
    use crate::ports::llm_gateway::RemoteModel;
    use crate::shared::base_url::BaseUrl;
    use crate::shared::net::NetworkScope;
    use crate::testing::{FixedClock, actor, remote_model};

    fn detected(remote: RemoteModel) -> DetectedModel {
        DetectedModel {
            remote,
            capabilities: Capabilities::default(),
        }
    }

    #[tokio::test]
    async fn sync_replaces_reported_limits_and_keeps_unreported_ones() {
        let clock = Arc::new(FixedClock::default());
        let store = MemoryStore::new(clock.clone());
        let user = actor().user_id;
        let url = BaseUrl::parse("https://api.example.com/v1", NetworkScope::Public).unwrap();
        let provider = Provider::create(
            user,
            ProviderKind::OpenAi,
            Preset::Custom,
            "Gateway",
            url,
            clock.now(),
        )
        .unwrap();
        ProviderRepository::insert(&store, &provider).await.unwrap();
        let mut reported = remote_model("m");
        reported.context_window = Some(100_000);
        store
            .sync(
                user,
                provider.id(),
                vec![detected(reported.clone())],
                false,
                10,
            )
            .await
            .unwrap();
        let model = ModelRepository::list(&store, user, ModelQuery::default())
            .await
            .unwrap()
            .remove(0);
        assert!(
            !model.enabled(),
            "auto_enable = false leaves new models off"
        );
        let edited = model.with_limits(Some(100_000), Some(8_000));
        ModelRepository::update(&store, &edited).await.unwrap();

        reported.context_window = Some(200_000);
        store
            .sync(user, provider.id(), vec![detected(reported)], true, 10)
            .await
            .unwrap();
        let model = ModelRepository::get(&store, user, edited.id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(model.context_window(), Some(200_000), "reported wins");
        assert_eq!(model.max_output_tokens(), Some(8_000), "unreported stays");
        assert!(!model.enabled(), "auto_enable never touches known models");
    }

    #[tokio::test]
    async fn sync_drops_spare_models_past_what_a_connection_keeps() {
        let clock = Arc::new(FixedClock::default());
        let store = MemoryStore::new(clock.clone());
        let user = actor().user_id;
        let url = BaseUrl::parse("https://api.example.com/v1", NetworkScope::Public).unwrap();
        let provider = Provider::create(
            user,
            ProviderKind::OpenAi,
            Preset::Custom,
            "Gateway",
            url,
            clock.now(),
        )
        .unwrap();
        ProviderRepository::insert(&store, &provider).await.unwrap();
        let listed = |ids: &[&str]| -> Vec<DetectedModel> {
            ids.iter().map(|id| detected(remote_model(id))).collect()
        };
        let remote_ids = || async {
            let mut ids: Vec<String> = ModelRepository::list(&store, user, ModelQuery::default())
                .await
                .unwrap()
                .iter()
                .map(|model| model.remote_id().to_owned())
                .collect();
            ids.sort();
            ids
        };
        store
            .sync(user, provider.id(), listed(&["a", "b"]), true, 3)
            .await
            .unwrap();
        clock.advance(chrono::Duration::seconds(1));
        store
            .sync(user, provider.id(), listed(&["c"]), false, 3)
            .await
            .unwrap();
        // a and b are unavailable but on: nothing is spare yet.
        assert_eq!(remote_ids().await, ["a", "b", "c"]);
        for model in ModelRepository::list(&store, user, ModelQuery::default())
            .await
            .unwrap()
        {
            ModelRepository::update(&store, &model.with_enabled(false))
                .await
                .unwrap();
        }
        clock.advance(chrono::Duration::seconds(1));
        store
            .sync(user, provider.id(), listed(&["d", "e"]), false, 3)
            .await
            .unwrap();
        // Five held, three kept: the spare a and b were updated least recently.
        assert_eq!(remote_ids().await, ["c", "d", "e"]);
    }
}
