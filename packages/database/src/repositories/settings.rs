//! `SettingsRepository` on Postgres (`user_settings`).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use copilot_core::{DomainResult, SettingsRepository, UserId, UserSettings};
use uuid::Uuid;

use crate::repositories::store::{PgStore, failed};

#[derive(sqlx::FromRow)]
struct SettingsRow {
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    user_id: Uuid,
    instructions: String,
}

#[async_trait]
impl SettingsRepository for PgStore {
    async fn get(&self, user: UserId) -> DomainResult<Option<UserSettings>> {
        let row: Option<SettingsRow> = sqlx::query_as(
            "SELECT created_at, updated_at, user_id, instructions FROM user_settings \
             WHERE user_id = $1",
        )
        .bind(user.uuid())
        .fetch_optional(self.pool())
        .await
        .map_err(failed("database.settings.get.failed"))?;
        Ok(row.map(|row| {
            UserSettings::restore(
                UserId::new(row.user_id),
                row.instructions,
                row.created_at,
                row.updated_at,
            )
        }))
    }

    async fn save(&self, settings: &UserSettings) -> DomainResult<()> {
        sqlx::query(
            "INSERT INTO user_settings (created_at, updated_at, user_id, instructions) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (user_id) DO UPDATE \
             SET updated_at = EXCLUDED.updated_at, instructions = EXCLUDED.instructions",
        )
        .bind(settings.created_at())
        .bind(settings.updated_at())
        .bind(settings.user_id().uuid())
        .bind(settings.instructions())
        .execute(self.pool())
        .await
        .map_err(failed("database.settings.save.failed"))?;
        Ok(())
    }
}
