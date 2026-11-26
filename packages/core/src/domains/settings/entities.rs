//! Personal settings of a user.

use chrono::{DateTime, Utc};

use crate::shared::ids::UserId;
use crate::shared::text;

/// A user's personal settings (`user_settings`). Today that is the personal instructions that
/// apply to every chat of the user. Accounts and passwords are lib-rustauth's.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserSettings {
    user_id: UserId,
    instructions: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl UserSettings {
    /// Settings of a user who never saved any: empty instructions.
    pub fn empty(user_id: UserId, now: DateTime<Utc>) -> Self {
        Self {
            user_id,
            instructions: String::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// A row read back from storage.
    pub fn restore(
        user_id: UserId,
        instructions: String,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            user_id,
            instructions,
            created_at,
            updated_at,
        }
    }

    /// A copy with new instructions (trimmed).
    #[must_use]
    pub fn with_instructions(mut self, instructions: &str, now: DateTime<Utc>) -> Self {
        self.instructions = text::trimmed(instructions);
        self.updated_at = now;
        self
    }

    pub fn user_id(&self) -> UserId {
        self.user_id
    }

    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};
    use uuid::Uuid;

    use super::*;

    #[test]
    fn starts_empty_and_keeps_created_at_on_change() {
        let user = UserId::new(Uuid::now_v7());
        let created = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let later = created + Duration::hours(1);
        let settings = UserSettings::empty(user, created).with_instructions("  Be brief. ", later);
        assert_eq!(settings.instructions(), "Be brief.");
        assert_eq!(settings.created_at(), created);
        assert_eq!(settings.updated_at(), later);
        assert_eq!(settings.user_id(), user);
    }
}
