//! The caller of a service.

use crate::shared::ids::UserId;

/// Who is acting: built by `apps/api` from lib-rustauth's `AuthUser` for a request, or by
/// `ScheduleRepository::due` for a scheduled run. Core never sees rustauth types.
///
/// `is_admin` is true only for the rustauth role `administrator`; `manager` acts as a plain
/// user in v1. Services read `is_admin` here and pass `user_id` to repositories.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Actor {
    pub user_id: UserId,
    pub is_admin: bool,
}

impl Actor {
    pub const fn new(user_id: UserId, is_admin: bool) -> Self {
        Self { user_id, is_admin }
    }

    /// A plain user.
    pub const fn user(user_id: UserId) -> Self {
        Self::new(user_id, false)
    }

    /// An administrator.
    pub const fn admin(user_id: UserId) -> Self {
        Self::new(user_id, true)
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn builds_users_and_admins() {
        let id = UserId::new(Uuid::now_v7());
        assert!(!Actor::user(id).is_admin);
        assert!(Actor::admin(id).is_admin);
        assert_eq!(Actor::admin(id).user_id, id);
    }
}
