//! Settings persistence port.

use async_trait::async_trait;

use crate::domains::settings::entities::UserSettings;
use crate::shared::errors::DomainResult;
use crate::shared::ids::UserId;

#[async_trait]
pub trait SettingsRepository: Send + Sync {
    /// `None` when the user never saved settings; that reads as empty instructions.
    async fn get(&self, user: UserId) -> DomainResult<Option<UserSettings>>;

    /// Upsert on `user_id`.
    async fn save(&self, settings: &UserSettings) -> DomainResult<()>;
}
