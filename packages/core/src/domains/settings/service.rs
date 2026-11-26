//! Personal settings use cases.

use std::sync::Arc;

use crate::domains::settings::entities::UserSettings;
use crate::domains::settings::ports::SettingsRepository;
use crate::ports::clock::Clock;
use crate::shared::actor::Actor;
use crate::shared::errors::DomainResult;

/// The caller's personal settings. Account, session, and user administration operations are
/// lib-rustauth's `AuthService` and `AdminService`.
#[derive(Clone)]
pub struct SettingsService {
    settings: Arc<dyn SettingsRepository>,
    clock: Arc<dyn Clock>,
}

impl SettingsService {
    pub fn new(settings: Arc<dyn SettingsRepository>, clock: Arc<dyn Clock>) -> Self {
        Self { settings, clock }
    }

    /// The caller's settings; empty instructions when no row exists yet.
    pub async fn get(&self, actor: &Actor) -> DomainResult<UserSettings> {
        let stored = self.settings.get(actor.user_id).await?;
        Ok(stored.unwrap_or_else(|| UserSettings::empty(actor.user_id, self.clock.now())))
    }

    /// Replaces the personal instructions (upsert). The first save creates the row, so
    /// registration in lib-rustauth needs no hook into Copilot.
    pub async fn update(&self, actor: &Actor, instructions: &str) -> DomainResult<UserSettings> {
        let now = self.clock.now();
        let current = self.settings.get(actor.user_id).await?;
        let settings = current
            .unwrap_or_else(|| UserSettings::empty(actor.user_id, now))
            .with_instructions(instructions, now);
        self.settings.save(&settings).await?;
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::testing::{FixedClock, MemoryStore, actor};

    fn service() -> (SettingsService, Arc<MemoryStore>, Arc<FixedClock>) {
        let clock = Arc::new(FixedClock::default());
        let store = Arc::new(MemoryStore::new(clock.clone()));
        (
            SettingsService::new(store.clone(), clock.clone()),
            store,
            clock,
        )
    }

    #[tokio::test]
    async fn reads_empty_instructions_without_a_row() {
        let (service, store, clock) = service();
        let caller = actor();
        let settings = service.get(&caller).await.unwrap();
        assert_eq!(settings.instructions(), "");
        assert_eq!(settings.user_id(), caller.user_id);
        assert_eq!(settings.created_at(), clock.now());
        let stored = SettingsRepository::get(store.as_ref(), caller.user_id)
            .await
            .unwrap();
        assert!(stored.is_none(), "reading never writes a row");
    }

    #[tokio::test]
    async fn creates_the_row_on_first_update_and_keeps_created_at() {
        let (service, _store, clock) = service();
        let caller = actor();
        let created = clock.now();
        service
            .update(&caller, "  Answer in French.  ")
            .await
            .unwrap();

        clock.advance(Duration::minutes(5));
        let updated = service.update(&caller, "Answer in German.").await.unwrap();
        assert_eq!(updated.instructions(), "Answer in German.");
        assert_eq!(updated.created_at(), created);
        assert_eq!(updated.updated_at(), clock.now());

        let read = service.get(&caller).await.unwrap();
        assert_eq!(read, updated);
    }

    #[tokio::test]
    async fn keeps_users_apart() {
        let (service, _store, _clock) = service();
        let alice = actor();
        let bob = actor();
        service.update(&alice, "Alice's rules").await.unwrap();
        assert_eq!(service.get(&bob).await.unwrap().instructions(), "");
        assert_eq!(
            service.get(&alice).await.unwrap().instructions(),
            "Alice's rules"
        );
    }
}
