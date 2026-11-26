//! `ToolServerRepository` on Postgres (`tool_servers`). The walking skeleton reads servers and
//! inserts them; roadmap step P2.A writes `update`, `delete`, and `save_listing`.

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use copilot_core::{
    BaseUrl, DomainError, DomainResult, Slug, ToolListing, ToolServer, ToolServerId,
    ToolServerParts, ToolServerRepository, UserId,
};
use serde_json::Value;
use sqlx::types::Json;
use uuid::Uuid;

use crate::records::{ToolCacheRecord, modes, to_modes};
use crate::repositories::store::{PgStore, failed, invalid, json, pending, record, text};

const COLUMNS: &str = "created_at, updated_at, tools_fetched_at, id, user_id, enabled, \
     tools_review_required, name, slug, url, approval, headers_sealed, protocol_version, \
     tool_modes, tools_cached";

#[derive(sqlx::FromRow)]
struct ServerRow {
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    tools_fetched_at: Option<DateTime<Utc>>,
    id: Uuid,
    user_id: Uuid,
    enabled: bool,
    tools_review_required: bool,
    name: String,
    slug: String,
    url: String,
    approval: String,
    headers_sealed: Option<Vec<u8>>,
    protocol_version: Option<String>,
    tool_modes: Json<BTreeMap<String, String>>,
    tools_cached: Option<Value>,
}

impl TryFrom<ServerRow> for ToolServer {
    type Error = DomainError;

    fn try_from(row: ServerRow) -> DomainResult<Self> {
        let cache = match (row.tools_cached, row.tools_fetched_at) {
            (Some(cached), Some(fetched_at)) => {
                let cached: ToolCacheRecord =
                    record(cached, "database.tool-servers.tools-cached.invalid")?;
                Some(
                    cached
                        .to_cache(fetched_at)
                        .map_err(invalid("database.tool-servers.tools-cached.invalid"))?,
                )
            }
            _ => None,
        };
        Ok(Self::restore(ToolServerParts {
            id: ToolServerId::new(row.id),
            user_id: UserId::new(row.user_id),
            name: row.name,
            slug: Slug::restore(row.slug),
            url: BaseUrl::restore(&row.url)?,
            headers_sealed: row.headers_sealed,
            approval: text(&row.approval, "database.tool-servers.approval.invalid")?,
            tool_modes: to_modes(row.tool_modes.0)
                .map_err(invalid("database.tool-servers.tool-modes.invalid"))?,
            enabled: row.enabled,
            protocol_version: row.protocol_version,
            cache,
            review_required: row.tools_review_required,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }))
    }
}

#[async_trait]
impl ToolServerRepository for PgStore {
    async fn list(&self, user: UserId) -> DomainResult<Vec<ToolServer>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM tool_servers WHERE user_id = $1 ORDER BY created_at, id"
        );
        let rows: Vec<ServerRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(user.uuid())
            .fetch_all(self.pool())
            .await
            .map_err(failed("database.tool-servers.list.failed"))?;
        rows.into_iter().map(ToolServer::try_from).collect()
    }

    async fn get(&self, user: UserId, id: ToolServerId) -> DomainResult<Option<ToolServer>> {
        let sql = format!("SELECT {COLUMNS} FROM tool_servers WHERE id = $1 AND user_id = $2");
        let row: Option<ServerRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(id.uuid())
            .bind(user.uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(failed("database.tool-servers.get.failed"))?;
        row.map(ToolServer::try_from).transpose()
    }

    async fn insert(&self, server: &ToolServer) -> DomainResult<()> {
        let cache = server.cache();
        sqlx::query(
            "INSERT INTO tool_servers (created_at, updated_at, tools_fetched_at, id, user_id, \
             enabled, tools_review_required, name, slug, url, approval, headers_sealed, \
             protocol_version, tool_modes, tools_cached) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
        )
        .bind(server.created_at())
        .bind(server.updated_at())
        .bind(cache.map(|cache| cache.fetched_at))
        .bind(server.id().uuid())
        .bind(server.user_id().uuid())
        .bind(server.enabled())
        .bind(server.review_required())
        .bind(server.name())
        .bind(server.slug().as_str())
        .bind(server.url().as_str())
        .bind(server.approval().as_str())
        .bind(server.headers_sealed())
        .bind(server.protocol_version())
        .bind(json(modes(server.tool_modes())))
        .bind(cache.map(|cache| json(ToolCacheRecord::from(cache))))
        .execute(self.pool())
        .await
        .map_err(failed("database.tool-servers.insert.failed"))?;
        Ok(())
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
