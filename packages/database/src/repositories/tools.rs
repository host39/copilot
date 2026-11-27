//! `ToolServerRepository` on Postgres (`tool_servers`) and `SignInRepository` (`oauth_flows`).
//! A server is at a `url` (Streamable HTTP) or runs a program (`command`, `args`, `env_sealed`,
//! `cwd`; stdio); `transport` says which and never changes.
//! Deleting a server cascades to its `project_tool_servers`, `chat_tool_servers`,
//! `chat_tool_approvals`, and `oauth_flows` rows. Only the user's review clears
//! `tools_review_required`: a listing saved while a settings write was in flight keeps the flag
//! it set, so a changed tool list always asks until the user has seen it. Only `save_sign_in`
//! writes a sign-in: a settings write keeps the stored one unless it leaves `oauth` or changes
//! the URL, so it never undoes a sign-in or a refresh that ran meanwhile.

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use copilot_core::{
    AuthMode, BaseUrl, ClientConfig, DomainError, DomainResult, Location, SignIn, SignInFlow,
    SignInRepository, Slug, StdioCommand, ToolCache, ToolListing, ToolServer, ToolServerId,
    ToolServerParts, ToolServerRepository, Transport, UserId,
};
use serde_json::Value;
use sqlx::types::Json;
use uuid::Uuid;

use crate::records::{ToolCacheRecord, modes, to_modes};
use crate::repositories::store::{PgStore, failed, invalid, json, record, text};

const COLUMNS: &str = "created_at, updated_at, tools_fetched_at, oauth_expires_at, id, user_id, \
     enabled, tools_review_required, oauth_refreshable, name, slug, url, approval, headers_sealed, \
     protocol_version, tool_modes, tools_cached, auth_mode, oauth_client_id, \
     oauth_client_secret_sealed, oauth_tokens_sealed, transport, command, env_sealed, cwd, args";

/// The SQL condition that the row's server is where the parameters say: the URL at `$url`, the
/// program's `command`, `args`, `env_sealed`, and `cwd` at `$program` to `$program + 3` (null for
/// the other transport). The sealed environment counts too: a new one may change the tools.
fn same_place(url: usize, program: usize) -> String {
    format!(
        "url IS NOT DISTINCT FROM ${url} AND command IS NOT DISTINCT FROM ${program} \
         AND args IS NOT DISTINCT FROM ${} AND env_sealed IS NOT DISTINCT FROM ${} \
         AND cwd IS NOT DISTINCT FROM ${}",
        program + 1,
        program + 2,
        program + 3,
    )
}

#[derive(sqlx::FromRow)]
struct ServerRow {
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    tools_fetched_at: Option<DateTime<Utc>>,
    oauth_expires_at: Option<DateTime<Utc>>,
    id: Uuid,
    user_id: Uuid,
    enabled: bool,
    tools_review_required: bool,
    oauth_refreshable: bool,
    name: String,
    slug: String,
    url: Option<String>,
    approval: String,
    headers_sealed: Option<Vec<u8>>,
    protocol_version: Option<String>,
    tool_modes: Json<BTreeMap<String, String>>,
    tools_cached: Option<Value>,
    auth_mode: String,
    oauth_client_id: Option<String>,
    oauth_client_secret_sealed: Option<Vec<u8>>,
    oauth_tokens_sealed: Option<Vec<u8>>,
    transport: String,
    command: Option<String>,
    env_sealed: Option<Vec<u8>>,
    cwd: Option<String>,
    args: Option<Vec<String>>,
}

impl ServerRow {
    /// The URL, or the program; the `tool_servers_transport_check_columns` constraint keeps them
    /// apart.
    fn location(&mut self) -> DomainResult<Location> {
        let transport =
            text::<Transport>(&self.transport, "database.tool-servers.transport.invalid")?;
        let invalid = || DomainError::Internal {
            key: "database.tool-servers.location.invalid".to_owned(),
            source: None,
        };
        match transport {
            Transport::Http => {
                let url = self.url.take().ok_or_else(invalid)?;
                Ok(Location::Http(BaseUrl::restore(&url)?))
            }
            Transport::Stdio => Ok(Location::Stdio(StdioCommand {
                command: self.command.take().ok_or_else(invalid)?,
                args: self.args.take().ok_or_else(invalid)?,
                env_sealed: self.env_sealed.take(),
                cwd: self.cwd.take().ok_or_else(invalid)?,
            })),
        }
    }
}

impl TryFrom<ServerRow> for ToolServer {
    type Error = DomainError;

    fn try_from(mut row: ServerRow) -> DomainResult<Self> {
        let location = row.location()?;
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
            location,
            headers_sealed: row.headers_sealed,
            auth: text::<AuthMode>(&row.auth_mode, "database.tool-servers.auth-mode.invalid")?,
            client: row.oauth_client_id.map(|id| ClientConfig {
                id,
                secret_sealed: row.oauth_client_secret_sealed,
            }),
            sign_in: row.oauth_tokens_sealed.map(|sealed| SignIn {
                sealed,
                expires_at: row.oauth_expires_at,
                refreshable: row.oauth_refreshable,
            }),
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
        let client = server.client();
        let sign_in = server.sign_in();
        let stdio = server.stdio();
        sqlx::query(
            "INSERT INTO tool_servers (created_at, updated_at, tools_fetched_at, id, user_id, \
             enabled, tools_review_required, name, slug, url, approval, headers_sealed, \
             protocol_version, tool_modes, tools_cached, auth_mode, oauth_client_id, \
             oauth_client_secret_sealed, oauth_tokens_sealed, oauth_expires_at, \
             oauth_refreshable, transport, command, args, env_sealed, cwd) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, \
             $17, $18, $19, $20, $21, $22, $23, $24, $25, $26)",
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
        .bind(server.url().map(BaseUrl::as_str))
        .bind(server.approval().as_str())
        .bind(server.headers_sealed())
        .bind(server.protocol_version())
        .bind(json(modes(server.tool_modes())))
        .bind(cache.map(|cache| json(ToolCacheRecord::from(cache))))
        .bind(server.auth().as_str())
        .bind(client.map(|client| client.id.as_str()))
        .bind(client.and_then(|client| client.secret_sealed.as_deref()))
        .bind(sign_in.map(|sign_in| sign_in.sealed.as_slice()))
        .bind(sign_in.and_then(|sign_in| sign_in.expires_at))
        .bind(sign_in.is_some_and(|sign_in| sign_in.refreshable))
        .bind(server.transport().as_str())
        .bind(stdio.map(|stdio| stdio.command.as_str()))
        .bind(stdio.map(|stdio| stdio.args.as_slice()))
        .bind(stdio.and_then(|stdio| stdio.env_sealed.as_deref()))
        .bind(stdio.map(|stdio| stdio.cwd.as_str()))
        .execute(self.pool())
        .await
        .map_err(failed("database.tool-servers.insert.failed"))?;
        Ok(())
    }

    async fn update(&self, server: &ToolServer) -> DomainResult<()> {
        // A listing saved since `server` was read (its `tools_fetched_at` moved) wins: it keeps
        // its review flag, and its protocol version while the server stays where it was (the
        // same URL, or the same program). The stored sign-in stays while the mode stays `oauth`
        // and the URL the same (a column in an expression is the row's old value). The
        // transport never changes, so a row of the other transport is left alone.
        let client = server.client();
        let stdio = server.stdio();
        let same = same_place(6, 16);
        let sql = format!(
            "UPDATE tool_servers SET updated_at = $3, enabled = $4, name = $5, url = $6, \
             approval = $7, headers_sealed = $8, tool_modes = $9, \
             tools_review_required = $10 \
               OR (tools_review_required AND tools_fetched_at IS DISTINCT FROM $11), \
             protocol_version = CASE \
               WHEN tools_fetched_at IS DISTINCT FROM $11 AND ({same}) THEN protocol_version \
               ELSE $12 END, \
             auth_mode = $13, oauth_client_id = $14, oauth_client_secret_sealed = $15, \
             oauth_tokens_sealed = CASE WHEN $13 = 'oauth' AND url = $6 \
               THEN oauth_tokens_sealed END, \
             oauth_expires_at = CASE WHEN $13 = 'oauth' AND url = $6 \
               THEN oauth_expires_at END, \
             oauth_refreshable = $13 = 'oauth' AND url = $6 AND oauth_refreshable, \
             command = $16, args = $17, env_sealed = $18, cwd = $19 \
             WHERE id = $1 AND user_id = $2 AND transport = $20"
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(server.id().uuid())
            .bind(server.user_id().uuid())
            .bind(server.updated_at())
            .bind(server.enabled())
            .bind(server.name())
            .bind(server.url().map(BaseUrl::as_str))
            .bind(server.approval().as_str())
            .bind(server.headers_sealed())
            .bind(json(modes(server.tool_modes())))
            .bind(server.review_required())
            .bind(server.cache().map(|cache| cache.fetched_at))
            .bind(server.protocol_version())
            .bind(server.auth().as_str())
            .bind(client.map(|client| client.id.as_str()))
            .bind(client.and_then(|client| client.secret_sealed.as_deref()))
            .bind(stdio.map(|stdio| stdio.command.as_str()))
            .bind(stdio.map(|stdio| stdio.args.as_slice()))
            .bind(stdio.and_then(|stdio| stdio.env_sealed.as_deref()))
            .bind(stdio.map(|stdio| stdio.cwd.as_str()))
            .bind(server.transport().as_str())
            .execute(self.pool())
            .await
            .map_err(failed("database.tool-servers.update.failed"))?;
        Ok(())
    }

    async fn delete(&self, user: UserId, id: ToolServerId) -> DomainResult<bool> {
        let deleted = sqlx::query("DELETE FROM tool_servers WHERE id = $1 AND user_id = $2")
            .bind(id.uuid())
            .bind(user.uuid())
            .execute(self.pool())
            .await
            .map_err(failed("database.tool-servers.delete.failed"))?
            .rows_affected();
        Ok(deleted > 0)
    }

    async fn save_listing(
        &self,
        user: UserId,
        id: ToolServerId,
        location: &Location,
        listing: &ToolListing,
        review_required: bool,
        at: DateTime<Utc>,
    ) -> DomainResult<bool> {
        let record = ToolCacheRecord::from(&ToolCache {
            tools: listing.tools.clone(),
            ttl: listing.ttl,
            fetched_at: at,
        });
        // Never clears the review flag: a URL change or another listing may have set it since
        // the caller read the server. Never stores the list of a URL or program the server no
        // longer has.
        let (url, stdio) = match location {
            Location::Http(url) => (Some(url.as_str()), None),
            Location::Stdio(command) => (None, Some(command)),
        };
        let sql = format!(
            "UPDATE tool_servers SET tools_cached = $3, tools_fetched_at = $4, \
             protocol_version = $5, tools_review_required = tools_review_required OR $6 \
             WHERE id = $1 AND user_id = $2 AND {}",
            same_place(7, 8)
        );
        let saved = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(id.uuid())
            .bind(user.uuid())
            .bind(json(record))
            .bind(at)
            .bind(&listing.protocol_version)
            .bind(review_required)
            .bind(url)
            .bind(stdio.map(|stdio| stdio.command.as_str()))
            .bind(stdio.map(|stdio| stdio.args.as_slice()))
            .bind(stdio.and_then(|stdio| stdio.env_sealed.as_deref()))
            .bind(stdio.map(|stdio| stdio.cwd.as_str()))
            .execute(self.pool())
            .await
            .map_err(failed("database.tool-servers.save-listing.failed"))?
            .rows_affected();
        Ok(saved > 0)
    }

    async fn save_sign_in(
        &self,
        user: UserId,
        id: ToolServerId,
        url: &BaseUrl,
        sign_in: Option<&SignIn>,
        previous: Option<&[u8]>,
    ) -> DomainResult<bool> {
        let saved = sqlx::query(
            "UPDATE tool_servers SET oauth_tokens_sealed = $4, oauth_expires_at = $5, \
             oauth_refreshable = $6 \
             WHERE id = $1 AND user_id = $2 AND url = $3 AND auth_mode = 'oauth' \
               AND ($7::bytea IS NULL OR oauth_tokens_sealed = $7)",
        )
        .bind(id.uuid())
        .bind(user.uuid())
        .bind(url.as_str())
        .bind(sign_in.map(|sign_in| sign_in.sealed.as_slice()))
        .bind(sign_in.and_then(|sign_in| sign_in.expires_at))
        .bind(sign_in.is_some_and(|sign_in| sign_in.refreshable))
        .bind(previous)
        .execute(self.pool())
        .await
        .map_err(failed("database.tool-servers.save-sign-in.failed"))?
        .rows_affected();
        Ok(saved > 0)
    }

    async fn stale(&self, user: UserId, id: ToolServerId) -> DomainResult<()> {
        // A `ttl_ms` of 0 expires the list now; `tools_fetched_at` keeps telling when it came.
        sqlx::query(
            "UPDATE tool_servers SET tools_cached = jsonb_set(tools_cached, '{ttl_ms}', '0') \
             WHERE id = $1 AND user_id = $2 AND tools_cached IS NOT NULL",
        )
        .bind(id.uuid())
        .bind(user.uuid())
        .execute(self.pool())
        .await
        .map_err(failed("database.tool-servers.stale.failed"))?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct FlowRow {
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    id: Uuid,
    user_id: Uuid,
    tool_server_id: Uuid,
    state_hash: Vec<u8>,
    sealed: Vec<u8>,
}

impl From<FlowRow> for SignInFlow {
    fn from(row: FlowRow) -> Self {
        Self {
            id: row.id,
            user_id: UserId::new(row.user_id),
            server_id: ToolServerId::new(row.tool_server_id),
            state_hash: row.state_hash,
            sealed: row.sealed,
            created_at: row.created_at,
            expires_at: row.expires_at,
        }
    }
}

#[async_trait]
impl SignInRepository for PgStore {
    async fn insert(&self, flow: &SignInFlow) -> DomainResult<()> {
        let mut transaction = self
            .pool()
            .begin()
            .await
            .map_err(failed("database.oauth-flows.insert.failed"))?;
        sqlx::query("DELETE FROM oauth_flows WHERE expires_at <= $1")
            .bind(flow.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(failed("database.oauth-flows.purge.failed"))?;
        // The composite key only accepts a server of `user_id`; a new flow of the server
        // replaces the one that never came back.
        sqlx::query(
            "INSERT INTO oauth_flows (created_at, expires_at, id, user_id, tool_server_id, \
             state_hash, sealed) VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (tool_server_id) DO UPDATE SET created_at = EXCLUDED.created_at, \
             expires_at = EXCLUDED.expires_at, id = EXCLUDED.id, \
             state_hash = EXCLUDED.state_hash, sealed = EXCLUDED.sealed \
             WHERE oauth_flows.user_id = EXCLUDED.user_id",
        )
        .bind(flow.created_at)
        .bind(flow.expires_at)
        .bind(flow.id)
        .bind(flow.user_id.uuid())
        .bind(flow.server_id.uuid())
        .bind(&flow.state_hash)
        .bind(&flow.sealed)
        .execute(&mut *transaction)
        .await
        .map_err(failed("database.oauth-flows.insert.failed"))?;
        transaction
            .commit()
            .await
            .map_err(failed("database.oauth-flows.insert.failed"))
    }

    async fn take(&self, state_hash: &[u8]) -> DomainResult<Option<SignInFlow>> {
        let row: Option<FlowRow> = sqlx::query_as(
            "DELETE FROM oauth_flows WHERE state_hash = $1 RETURNING created_at, expires_at, \
             id, user_id, tool_server_id, state_hash, sealed",
        )
        .bind(state_hash)
        .fetch_optional(self.pool())
        .await
        .map_err(failed("database.oauth-flows.take.failed"))?;
        Ok(row.map(SignInFlow::from))
    }
}
