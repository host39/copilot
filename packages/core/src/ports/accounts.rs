//! Port for the accounts schedules run as. lib-rustauth owns accounts; the API adapts it.

use async_trait::async_trait;

use crate::shared::actor::Actor;
use crate::shared::errors::DomainResult;
use crate::shared::ids::UserId;

/// The accounts behind background work (scheduled runs), resolved many at once.
#[async_trait]
pub trait Accounts: Send + Sync {
    /// The actors of the accounts with these ids that may act now: a disabled or missing
    /// account is left out. Order is not kept.
    async fn active(&self, ids: &[UserId]) -> DomainResult<Vec<Actor>>;
}
