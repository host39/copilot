//! In-memory `ToolServerRepository` and `SignInRepository`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domains::chat::{Chat, ChatParts};
use crate::domains::tool::{
    AuthMode, SignIn, SignInFlow, SignInRepository, ToolServer, ToolServerParts,
    ToolServerRepository,
};
use crate::ports::tool_host::ToolListing;
use crate::shared::base_url::BaseUrl;
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
            // The listing columns (`tools_cached`, `tools_fetched_at`) belong to `save_listing`,
            // the sign-in to `save_sign_in`: it survives unless the mode or the URL changed.
            let sign_in = if server.auth() == AuthMode::OAuth && stored.url() == server.url() {
                stored.sign_in().cloned()
            } else {
                None
            };
            let updated = ToolServer::restore(ToolServerParts {
                cache: stored.cache().cloned(),
                sign_in,
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
        url: &BaseUrl,
        listing: &ToolListing,
        review_required: bool,
        at: DateTime<Utc>,
    ) -> DomainResult<bool> {
        let mut tables = self.tables();
        let Some(stored) = tables
            .tool_servers
            .get_mut(&id)
            .filter(|stored| stored.user_id() == user && stored.url() == url)
        else {
            return Ok(false);
        };
        // A set review flag stays: only the user's review clears it.
        let review_required = stored.review_required() || review_required;
        *stored = stored.clone().with_listing(listing, review_required, at);
        Ok(true)
    }

    async fn save_sign_in(
        &self,
        user: UserId,
        id: ToolServerId,
        url: &BaseUrl,
        sign_in: Option<&SignIn>,
        previous: Option<&[u8]>,
    ) -> DomainResult<bool> {
        let mut tables = self.tables();
        let Some(stored) = tables.tool_servers.get_mut(&id).filter(|stored| {
            stored.user_id() == user && stored.url() == url && stored.auth() == AuthMode::OAuth
        }) else {
            return Ok(false);
        };
        if let Some(previous) = previous
            && stored.sign_in().map(|stored| stored.sealed.as_slice()) != Some(previous)
        {
            return Ok(false);
        }
        *stored = stored.clone().with_sign_in(sign_in.cloned());
        Ok(true)
    }

    async fn stale(&self, user: UserId, id: ToolServerId) -> DomainResult<()> {
        let mut tables = self.tables();
        if let Some(stored) = tables
            .tool_servers
            .get_mut(&id)
            .filter(|stored| stored.user_id() == user)
        {
            *stored = stored.clone().stale();
        }
        Ok(())
    }
}

#[async_trait]
impl SignInRepository for MemoryStore {
    async fn insert(&self, flow: &SignInFlow) -> DomainResult<()> {
        let now = self.now();
        let mut tables = self.tables();
        tables.sign_in_flows.retain(|_, stored| {
            stored.expires_at > now
                && !(stored.user_id == flow.user_id && stored.server_id == flow.server_id)
        });
        tables.sign_in_flows.insert(flow.id, flow.clone());
        Ok(())
    }

    async fn take(&self, state_hash: &[u8]) -> DomainResult<Option<SignInFlow>> {
        let mut tables = self.tables();
        let id = tables
            .sign_in_flows
            .values()
            .find(|flow| flow.state_hash == state_hash)
            .map(|flow| flow.id);
        Ok(id.and_then(|id| tables.sign_in_flows.remove(&id)))
    }
}

impl Tables {
    /// Deletes a server and its join rows (`ON DELETE CASCADE` on `project_tool_servers` and
    /// `oauth_flows`).
    pub(crate) fn delete_tool_server(&mut self, id: ToolServerId) {
        self.tool_servers.remove(&id);
        self.sign_in_flows.retain(|_, flow| flow.server_id != id);
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
        for chat in self.chats.values_mut() {
            if chat.tool_overrides().iter().any(|row| row.server_id == id) {
                let kept = chat
                    .tool_overrides()
                    .iter()
                    .copied()
                    .filter(|row| row.server_id != id)
                    .collect();
                *chat = chat.clone().with_tool_overrides(kept);
            }
            if chat
                .approved_tools()
                .iter()
                .any(|tool| tool.server_id == id)
            {
                let kept = chat
                    .approved_tools()
                    .iter()
                    .filter(|tool| tool.server_id != id)
                    .cloned()
                    .collect();
                *chat = Chat::restore(ChatParts {
                    approved_tools: kept,
                    ..chat.to_parts()
                });
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
        auth: server.auth(),
        client: server.client().cloned(),
        sign_in: server.sign_in().cloned(),
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
