//! Domain layer: entities, services, repository ports, and shared errors.
//!
//! Business domains live under `src/domains/<name>/` and are re-exported here. See
//! `docs/adr/001-domain-driven-design.md` for the layout and `docs/design/copilot.md` (Domain
//! Model) for the rules.
//!
//! This crate must not depend on HTTP, database, crypto, logging, or identity implementations.
//! Callers arrive as an [`Actor`] that `apps/api` builds from lib-rustauth's user.

pub mod domains;
pub mod ports;
pub mod shared;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

// Domains
pub use crate::domains::settings::{SettingsRepository, SettingsService, UserSettings};
// Ports (interfaces implemented by adapters in other crates)
pub use crate::ports::clock::{Clock, SystemClock};
pub use crate::ports::logger::LogLevel;
pub use crate::ports::secret_cipher::{SECRET_UNREADABLE, SealContext, SecretCipher};
// Shared types
pub use crate::shared::actor::Actor;
pub use crate::shared::constants::{DEFAULT_PAGE_OFFSET, DEFAULT_PAGE_SIZE};
pub use crate::shared::errors::{BoxError, Context, DomainError, DomainResult};
pub use crate::shared::ids::{
    ChatId, FileId, MessageId, ModelId, ProjectId, ProviderId, ScheduleId, ToolServerId, UserId,
};
pub use crate::shared::secret::Secret;
pub use crate::shared::types::{
    Environment, PaginatedResult, PaginationOptions, SortDirection, SortOptions,
};
