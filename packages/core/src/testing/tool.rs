//! In-memory `ToolServerRepository`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domains::tool::{ToolServer, ToolServerParts, ToolServerRepository};
use crate::ports::tool_host::ToolListing;
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{ToolServerId, UserId};
use crate::testing::store::{MemoryStore, Tables, duplicate};

#[async_trait]
impl ToolServerRepository for MemoryStore {
    async fn list(&self, user: UserId) -> DomainResult<Vec<ToolServer>> {
        let tables = self.tables();
        Ok(tables
            .tool_servers
            .values()
            .filter(|server| server.user_id() == user)
            .cloned()
            .collect())
    }

    async fn get(&self, user: UserId, id: ToolServerId) -> DomainResult<Option<ToolServer>> {
        let tables = self.tables();
        Ok(tables
            .tool_servers
            .get(&id)
            .filter(|server| server.user_id() == user)
            .cloned())
    }

    async fn insert(&self, server: &ToolServer) -> DomainResult<()> {
        let mut tables = self.tables();
        if tables.tool_servers.contains_key(&server.id()) {
            return Err(duplicate("tool-servers"));
        }
        let slug_taken = tables
            .tool_servers
            .values()
            .any(|stored| stored.user_id() == server.user_id() && stored.slug() == server.slug());
        if slug_taken {
            return Err(DomainError::Internal {
                key: "testing.tool-servers.insert.slug-taken".to_owned(),
                source: None,
            });
        }
        tables.tool_servers.insert(server.id(), server.clone());
        Ok(())
    }

    async fn update(&self, server: &ToolServer) -> DomainResult<()> {
        let mut tables = self.tables();
        if let Some(stored) = tables.tool_servers.get_mut(&server.id())
            && stored.user_id() == server.user_id()
        {
            // The listing columns (`tools_cached`, `tools_fetched_at`) belong to `save_listing`.
            let updated = ToolServer::restore(ToolServerParts {
                cache: stored.cache().cloned(),
                ..parts(server)
            });
            *stored = updated;
        }
        Ok(())
    }

    async fn delete(&self, user: UserId, id: ToolServerId) -> DomainResult<bool> {
        let mut tables = self.tables();
        if !tables
            .tool_servers
            .get(&id)
            .is_some_and(|server| server.user_id() == user)
        {
            return Ok(false);
        }
        tables.delete_tool_server(id);
        Ok(true)
    }

    async fn save_listing(
        &self,
        user: UserId,
        id: ToolServerId,
        listing: &ToolListing,
        review_required: bool,
        at: DateTime<Utc>,
    ) -> DomainResult<()> {
        let mut tables = self.tables();
        if let Some(stored) = tables.tool_servers.get_mut(&id)
            && stored.user_id() == user
        {
            *stored = stored.clone().with_listing(listing, review_required, at);
        }
        Ok(())
    }
}

impl Tables {
    /// Deletes a server and its join rows (`ON DELETE CASCADE` on `project_tool_servers`).
    pub(crate) fn delete_tool_server(&mut self, id: ToolServerId) {
        self.tool_servers.remove(&id);
        for project in self.projects.values_mut() {
            if project.tool_server_ids().contains(&id) {
                let kept = project
                    .tool_server_ids()
                    .iter()
                    .copied()
                    .filter(|server| *server != id)
                    .collect();
                *project = project.clone().with_tool_servers(kept);
            }
        }
    }
}

fn parts(server: &ToolServer) -> ToolServerParts {
    ToolServerParts {
        id: server.id(),
        user_id: server.user_id(),
        name: server.name().to_owned(),
        slug: server.slug().clone(),
        url: server.url().clone(),
        headers_sealed: server.headers_sealed().map(<[u8]>::to_vec),
        approval: server.approval(),
        tool_modes: server.tool_modes().clone(),
        enabled: server.enabled(),
        protocol_version: server.protocol_version().map(str::to_owned),
        cache: server.cache().cloned(),
        review_required: server.review_required(),
        created_at: server.created_at(),
        updated_at: server.updated_at(),
    }
}
