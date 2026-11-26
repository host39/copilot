//! In-memory `SettingsRepository`.

use async_trait::async_trait;

use crate::domains::settings::{SettingsRepository, UserSettings};
use crate::shared::errors::DomainResult;
use crate::shared::ids::UserId;
use crate::testing::store::MemoryStore;

#[async_trait]
impl SettingsRepository for MemoryStore {
    async fn get(&self, user: UserId) -> DomainResult<Option<UserSettings>> {
        Ok(self.tables().settings.get(&user).cloned())
    }

    async fn save(&self, settings: &UserSettings) -> DomainResult<()> {
        self.tables()
            .settings
            .insert(settings.user_id(), settings.clone());
        Ok(())
    }
}
