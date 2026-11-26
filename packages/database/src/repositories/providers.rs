//! `ProviderRepository` and `ModelRepository` on Postgres (`providers`, `models`). Roadmap step
//! P2.A writes the queries.

use async_trait::async_trait;
use copilot_core::{
    DetectedModel, DomainResult, Model, ModelId, ModelQuery, ModelRepository, Provider, ProviderId,
    ProviderRepository, ProviderUsage, SyncReport, UserId,
};

use crate::repositories::store::{PgStore, pending};

#[async_trait]
impl ProviderRepository for PgStore {
    async fn list(&self, _user: UserId) -> DomainResult<Vec<Provider>> {
        pending("ProviderRepository::list")
    }

    async fn get(&self, _user: UserId, _id: ProviderId) -> DomainResult<Option<Provider>> {
        pending("ProviderRepository::get")
    }

    async fn insert(&self, _provider: &Provider) -> DomainResult<()> {
        pending("ProviderRepository::insert")
    }

    async fn update(&self, _provider: &Provider) -> DomainResult<()> {
        pending("ProviderRepository::update")
    }

    async fn delete(&self, _user: UserId, _id: ProviderId) -> DomainResult<bool> {
        pending("ProviderRepository::delete")
    }

    async fn usage(&self, _user: UserId, _id: ProviderId) -> DomainResult<ProviderUsage> {
        pending("ProviderRepository::usage")
    }
}

#[async_trait]
impl ModelRepository for PgStore {
    async fn list(&self, _user: UserId, _query: ModelQuery) -> DomainResult<Vec<Model>> {
        pending("ModelRepository::list")
    }

    async fn get(&self, _user: UserId, _id: ModelId) -> DomainResult<Option<Model>> {
        pending("ModelRepository::get")
    }

    async fn sync(
        &self,
        _user: UserId,
        _provider: ProviderId,
        _remote: Vec<DetectedModel>,
        _auto_enable: bool,
    ) -> DomainResult<SyncReport> {
        pending("ModelRepository::sync")
    }

    async fn update(&self, _model: &Model) -> DomainResult<()> {
        pending("ModelRepository::update")
    }
}
