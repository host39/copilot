//! `SettingsRepository` on Postgres (`user_settings`). Roadmap step P2.A writes the queries.

use async_trait::async_trait;
use copilot_core::{DomainResult, SettingsRepository, UserId, UserSettings};

use crate::repositories::store::{PgStore, pending};

#[async_trait]
impl SettingsRepository for PgStore {
    async fn get(&self, _user: UserId) -> DomainResult<Option<UserSettings>> {
        pending("SettingsRepository::get")
    }

    async fn save(&self, _settings: &UserSettings) -> DomainResult<()> {
        pending("SettingsRepository::save")
    }
}
