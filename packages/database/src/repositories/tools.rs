//! `ToolServerRepository` on Postgres (`tool_servers`). Roadmap step P2.A writes the queries.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use copilot_core::{
    DomainResult, ToolListing, ToolServer, ToolServerId, ToolServerRepository, UserId,
};

use crate::repositories::store::{PgStore, pending};

#[async_trait]
impl ToolServerRepository for PgStore {
    async fn list(&self, _user: UserId) -> DomainResult<Vec<ToolServer>> {
        pending("ToolServerRepository::list")
    }

    async fn get(&self, _user: UserId, _id: ToolServerId) -> DomainResult<Option<ToolServer>> {
        pending("ToolServerRepository::get")
    }

    async fn insert(&self, _server: &ToolServer) -> DomainResult<()> {
        pending("ToolServerRepository::insert")
    }

    async fn update(&self, _server: &ToolServer) -> DomainResult<()> {
        pending("ToolServerRepository::update")
    }

    async fn delete(&self, _user: UserId, _id: ToolServerId) -> DomainResult<bool> {
        pending("ToolServerRepository::delete")
    }

    async fn save_listing(
        &self,
        _user: UserId,
        _id: ToolServerId,
        _listing: &ToolListing,
        _review_required: bool,
        _at: DateTime<Utc>,
    ) -> DomainResult<()> {
        pending("ToolServerRepository::save_listing")
    }
}
