//! `ProviderRepository` and `ModelRepository` on Postgres (`providers`, `models`).

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use copilot_core::{
    BaseUrl, Capabilities, CapabilityOverrides, DetectedModel, DomainError, DomainResult, Model,
    ModelId, ModelParts, ModelQuery, ModelRepository, Provider, ProviderId, ProviderParts,
    ProviderRepository, ProviderUsage, SyncReport, UserId,
};
use serde_json::Value;
use sqlx::types::Json;
use uuid::Uuid;

use crate::records::{CapabilitiesRecord, OverridesRecord};
use crate::repositories::store::{PgStore, failed, int, invalid, json, record, text, uint};

const PROVIDER_COLUMNS: &str = "created_at, updated_at, id, user_id, enabled, kind, preset, \
     name, base_url, api_key_sealed";

#[derive(sqlx::FromRow)]
struct ProviderRow {
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    id: Uuid,
    user_id: Uuid,
    enabled: bool,
    kind: String,
    preset: String,
    name: String,
    base_url: String,
    api_key_sealed: Option<Vec<u8>>,
}

impl TryFrom<ProviderRow> for Provider {
    type Error = DomainError;

    fn try_from(row: ProviderRow) -> DomainResult<Self> {
        Ok(Self::restore(ProviderParts {
            id: ProviderId::new(row.id),
            user_id: UserId::new(row.user_id),
            kind: text(&row.kind, "database.providers.kind.invalid")?,
            preset: text(&row.preset, "database.providers.preset.invalid")?,
            name: row.name,
            base_url: BaseUrl::restore(&row.base_url)?,
            api_key_sealed: row.api_key_sealed,
            enabled: row.enabled,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }))
    }
}

#[derive(sqlx::FromRow)]
struct ModelRow {
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    context_window: Option<i32>,
    max_output_tokens: Option<i32>,
    id: Uuid,
    user_id: Uuid,
    provider_id: Uuid,
    enabled: bool,
    available: bool,
    remote_id: String,
    display_name: String,
    capabilities: Value,
    capability_overrides: Value,
}

impl TryFrom<ModelRow> for Model {
    type Error = DomainError;

    fn try_from(row: ModelRow) -> DomainResult<Self> {
        let capabilities: CapabilitiesRecord =
            record(row.capabilities, "database.models.capabilities.invalid")?;
        let overrides: OverridesRecord = record(
            row.capability_overrides,
            "database.models.capability-overrides.invalid",
        )?;
        Ok(Self::restore(ModelParts {
            id: ModelId::new(row.id),
            user_id: UserId::new(row.user_id),
            provider_id: ProviderId::new(row.provider_id),
            remote_id: row.remote_id,
            display_name: row.display_name,
            enabled: row.enabled,
            available: row.available,
            context_window: uint(row.context_window),
            max_output_tokens: uint(row.max_output_tokens),
            capabilities: Capabilities::try_from(capabilities)
                .map_err(invalid("database.models.capabilities.invalid"))?,
            overrides: CapabilityOverrides::try_from(overrides)
                .map_err(invalid("database.models.capability-overrides.invalid"))?,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }))
    }
}

#[async_trait]
impl ProviderRepository for PgStore {
    async fn list(&self, user: UserId) -> DomainResult<Vec<Provider>> {
        let sql = format!(
            "SELECT {PROVIDER_COLUMNS} FROM providers WHERE user_id = $1 ORDER BY created_at, id"
        );
        let rows: Vec<ProviderRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(user.uuid())
            .fetch_all(self.pool())
            .await
            .map_err(failed("database.providers.list.failed"))?;
        rows.into_iter().map(Provider::try_from).collect()
    }

    async fn get(&self, user: UserId, id: ProviderId) -> DomainResult<Option<Provider>> {
        let sql =
            format!("SELECT {PROVIDER_COLUMNS} FROM providers WHERE id = $1 AND user_id = $2");
        let row: Option<ProviderRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(id.uuid())
            .bind(user.uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(failed("database.providers.get.failed"))?;
        row.map(Provider::try_from).transpose()
    }

    async fn insert(&self, provider: &Provider) -> DomainResult<()> {
        sqlx::query(
            "INSERT INTO providers (created_at, updated_at, id, user_id, enabled, kind, preset, \
             name, base_url, api_key_sealed) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(provider.created_at())
        .bind(provider.updated_at())
        .bind(provider.id().uuid())
        .bind(provider.user_id().uuid())
        .bind(provider.enabled())
        .bind(provider.kind().as_str())
        .bind(provider.preset().as_str())
        .bind(provider.name())
        .bind(provider.base_url().as_str())
        .bind(provider.api_key_sealed())
        .execute(self.pool())
        .await
        .map_err(failed("database.providers.insert.failed"))?;
        Ok(())
    }

    async fn update(&self, provider: &Provider) -> DomainResult<()> {
        sqlx::query(
            "UPDATE providers SET updated_at = $3, enabled = $4, name = $5, base_url = $6, \
             api_key_sealed = $7 WHERE id = $1 AND user_id = $2",
        )
        .bind(provider.id().uuid())
        .bind(provider.user_id().uuid())
        .bind(provider.updated_at())
        .bind(provider.enabled())
        .bind(provider.name())
        .bind(provider.base_url().as_str())
        .bind(provider.api_key_sealed())
        .execute(self.pool())
        .await
        .map_err(failed("database.providers.update.failed"))?;
        Ok(())
    }

    async fn delete(&self, user: UserId, id: ProviderId) -> DomainResult<bool> {
        let deleted = sqlx::query("DELETE FROM providers WHERE id = $1 AND user_id = $2")
            .bind(id.uuid())
            .bind(user.uuid())
            .execute(self.pool())
            .await
            .map_err(failed("database.providers.delete.failed"))?
            .rows_affected();
        Ok(deleted > 0)
    }

    async fn usage(&self, user: UserId, id: ProviderId) -> DomainResult<ProviderUsage> {
        // The composite foreign keys keep a row's model the row owner's, so joining on the model
        // id alone stays within `user`.
        let row: UsageRow = sqlx::query_as(
            "SELECT \
             (SELECT count(*) FROM chats c JOIN models m ON m.id = c.model_id \
               WHERE c.user_id = $1 AND m.provider_id = $2) AS chats, \
             (SELECT count(*) FROM projects p JOIN models m ON m.id = p.default_model_id \
               WHERE p.user_id = $1 AND m.provider_id = $2) AS projects, \
             (SELECT count(*) FROM schedules s JOIN models m ON m.id = s.model_id \
               WHERE s.user_id = $1 AND m.provider_id = $2) AS schedules",
        )
        .bind(user.uuid())
        .bind(id.uuid())
        .fetch_one(self.pool())
        .await
        .map_err(failed("database.providers.usage.failed"))?;
        Ok(ProviderUsage {
            chats: unsigned(row.chats),
            projects: unsigned(row.projects),
            schedules: unsigned(row.schedules),
        })
    }
}

/// The rows that name one of a connection's models.
#[derive(sqlx::FromRow)]
struct UsageRow {
    chats: i64,
    projects: i64,
    schedules: i64,
}

/// A `count(*)`, which is never negative.
fn unsigned(count: i64) -> u64 {
    u64::try_from(count).unwrap_or_default()
}

const MODEL_COLUMNS: &str = "m.created_at, m.updated_at, m.context_window, m.max_output_tokens, \
     m.id, m.user_id, m.provider_id, m.enabled, m.available, m.remote_id, m.display_name, \
     m.capabilities, m.capability_overrides";

/// Columns of the models a refresh updates or inserts, one array per column (`UNNEST`).
#[derive(Default)]
struct Batch {
    ids: Vec<Uuid>,
    remote_ids: Vec<String>,
    names: Vec<String>,
    capabilities: Vec<Json<CapabilitiesRecord>>,
    context_windows: Vec<Option<i32>>,
    output_limits: Vec<Option<i32>>,
}

fn count(rows: usize) -> u32 {
    u32::try_from(rows).unwrap_or(u32::MAX)
}

/// A stored model's limits, for a refresh that keeps the ones the provider no longer reports.
#[derive(sqlx::FromRow)]
struct KnownRow {
    id: Uuid,
    remote_id: String,
    available: bool,
}

#[async_trait]
impl ModelRepository for PgStore {
    async fn list(&self, user: UserId, query: ModelQuery) -> DomainResult<Vec<Model>> {
        let sql = format!(
            "SELECT {MODEL_COLUMNS} FROM models m \
             JOIN providers p ON p.id = m.provider_id AND p.user_id = m.user_id \
             WHERE m.user_id = $1 AND ($2::uuid IS NULL OR m.provider_id = $2) \
             AND (NOT $3 OR (m.enabled AND m.available AND p.enabled)) \
             ORDER BY m.provider_id, m.display_name, m.id"
        );
        let rows: Vec<ModelRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(user.uuid())
            .bind(query.provider.map(ProviderId::uuid))
            .bind(query.usable)
            .fetch_all(self.pool())
            .await
            .map_err(failed("database.models.list.failed"))?;
        rows.into_iter().map(Model::try_from).collect()
    }

    async fn get(&self, user: UserId, id: ModelId) -> DomainResult<Option<Model>> {
        let sql =
            format!("SELECT {MODEL_COLUMNS} FROM models m WHERE m.id = $1 AND m.user_id = $2");
        let row: Option<ModelRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(id.uuid())
            .bind(user.uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(failed("database.models.get.failed"))?;
        row.map(Model::try_from).transpose()
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
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(failed("database.models.sync.failed"))?;
        // The row lock serializes refreshes of one connection, so two of them never insert the
        // same remote model.
        let owned: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM providers WHERE id = $1 AND user_id = $2 FOR UPDATE",
        )
        .bind(provider.uuid())
        .bind(user.uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(failed("database.models.sync.failed"))?;
        if owned.is_none() {
            return Err(DomainError::Internal {
                key: "database.models.sync.provider-missing".to_owned(),
                source: None,
            });
        }
        let known: Vec<KnownRow> = sqlx::query_as(
            "SELECT id, remote_id, available FROM models WHERE provider_id = $1 AND user_id = $2",
        )
        .bind(provider.uuid())
        .bind(user.uuid())
        .fetch_all(&mut *tx)
        .await
        .map_err(failed("database.models.sync.failed"))?;
        let ids: HashMap<&str, Uuid> = known
            .iter()
            .map(|row| (row.remote_id.as_str(), row.id))
            .collect();

        // One statement per kind of change, whatever the number of models.
        let mut listed = Batch::default();
        let mut added = Batch::default();
        for detected in remote {
            let capabilities = json(CapabilitiesRecord::from(&detected.capabilities));
            let model = detected.remote;
            let batch = match ids.get(model.remote_id.as_str()) {
                Some(id) => {
                    listed.ids.push(*id);
                    &mut listed
                }
                None => {
                    added.ids.push(ModelId::generate().uuid());
                    added.remote_ids.push(model.remote_id);
                    added.names.push(model.display_name);
                    &mut added
                }
            };
            batch.capabilities.push(capabilities);
            batch.context_windows.push(int(model.context_window));
            batch.output_limits.push(int(model.max_output_tokens));
        }
        if !listed.ids.is_empty() {
            sqlx::query(
                "UPDATE models m SET updated_at = $2, available = true, \
                 capabilities = u.capabilities, \
                 context_window = COALESCE(u.context_window, m.context_window), \
                 max_output_tokens = COALESCE(u.max_output_tokens, m.max_output_tokens) \
                 FROM UNNEST($1::uuid[], $3::jsonb[], $4::integer[], $5::integer[]) \
                 AS u (id, capabilities, context_window, max_output_tokens) WHERE m.id = u.id",
            )
            .bind(&listed.ids)
            .bind(now)
            .bind(&listed.capabilities)
            .bind(&listed.context_windows)
            .bind(&listed.output_limits)
            .execute(&mut *tx)
            .await
            .map_err(failed("database.models.sync.failed"))?;
        }
        if !added.ids.is_empty() {
            sqlx::query(
                "INSERT INTO models (created_at, updated_at, context_window, max_output_tokens, \
                 id, user_id, provider_id, enabled, available, remote_id, display_name, \
                 capabilities, capability_overrides) \
                 SELECT $1, $1, u.context_window, u.max_output_tokens, u.id, $2, $3, $4, true, \
                 u.remote_id, u.display_name, u.capabilities, $5 \
                 FROM UNNEST($6::integer[], $7::integer[], $8::uuid[], $9::text[], $10::text[], \
                 $11::jsonb[]) \
                 AS u (context_window, max_output_tokens, id, remote_id, display_name, capabilities)",
            )
            .bind(now)
            .bind(user.uuid())
            .bind(provider.uuid())
            .bind(auto_enable)
            .bind(json(OverridesRecord::from(&CapabilityOverrides::default())))
            .bind(&added.context_windows)
            .bind(&added.output_limits)
            .bind(&added.ids)
            .bind(&added.remote_ids)
            .bind(&added.names)
            .bind(&added.capabilities)
            .execute(&mut *tx)
            .await
            .map_err(failed("database.models.sync.failed"))?;
        }

        let mut report = SyncReport {
            added: count(added.ids.len()),
            updated: count(listed.ids.len()),
            enabled: if auto_enable {
                count(added.ids.len())
            } else {
                0
            },
            ..SyncReport::default()
        };
        let still: HashSet<Uuid> = listed.ids.iter().copied().collect();
        let gone: Vec<Uuid> = known
            .iter()
            .filter(|row| row.available && !still.contains(&row.id))
            .map(|row| row.id)
            .collect();
        if !gone.is_empty() {
            sqlx::query("UPDATE models SET updated_at = $2, available = false WHERE id = ANY($1)")
                .bind(&gone)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(failed("database.models.sync.failed"))?;
            report.unavailable = count(gone.len());
        }
        let excess = (known.len() + added.ids.len()).saturating_sub(keep);
        if excess > 0 {
            sqlx::query(
                "DELETE FROM models WHERE id IN (SELECT m.id FROM models m \
                 WHERE m.provider_id = $1 AND m.user_id = $2 AND NOT m.available \
                 AND NOT m.enabled \
                 AND NOT EXISTS (SELECT 1 FROM chats c WHERE c.model_id = m.id) \
                 AND NOT EXISTS (SELECT 1 FROM messages x WHERE x.model_id = m.id) \
                 AND NOT EXISTS (SELECT 1 FROM projects p WHERE p.default_model_id = m.id) \
                 AND NOT EXISTS (SELECT 1 FROM schedules s WHERE s.model_id = m.id) \
                 ORDER BY m.updated_at, m.id LIMIT $3)",
            )
            .bind(provider.uuid())
            .bind(user.uuid())
            .bind(i64::try_from(excess).unwrap_or(i64::MAX))
            .execute(&mut *tx)
            .await
            .map_err(failed("database.models.sync.failed"))?;
        }
        tx.commit()
            .await
            .map_err(failed("database.models.sync.failed"))?;
        Ok(report)
    }

    async fn update(&self, model: &Model) -> DomainResult<()> {
        sqlx::query(
            "UPDATE models SET updated_at = $3, context_window = $4, max_output_tokens = $5, \
             enabled = $6, available = $7, display_name = $8, capabilities = $9, \
             capability_overrides = $10 WHERE id = $1 AND user_id = $2",
        )
        .bind(model.id().uuid())
        .bind(model.user_id().uuid())
        .bind(model.updated_at())
        .bind(int(model.context_window()))
        .bind(int(model.max_output_tokens()))
        .bind(model.enabled())
        .bind(model.available())
        .bind(model.display_name())
        .bind(json(CapabilitiesRecord::from(model.capabilities())))
        .bind(json(OverridesRecord::from(model.overrides())))
        .execute(self.pool())
        .await
        .map_err(failed("database.models.update.failed"))?;
        Ok(())
    }
}
