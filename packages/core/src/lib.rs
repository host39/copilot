//! Domain layer: entities, services, repository ports, and shared errors.
//!
//! Add business domains under `src/domains/<name>/` and re-export them here.
//! See `docs/adr/001-domain-driven-design.md` for the layout and workflow.
//!
//! This crate must not depend on HTTP, database, or logging implementations.

pub mod domains;
pub mod ports;
pub mod shared;

// Ports (interfaces implemented by adapters in other crates)
pub use crate::ports::logger::LogLevel;
// Shared errors and types
pub use crate::shared::constants::{DEFAULT_PAGE_OFFSET, DEFAULT_PAGE_SIZE};
pub use crate::shared::errors::{BoxError, Context, DomainError, DomainResult};
pub use crate::shared::types::{
    Environment, PaginatedResult, PaginationOptions, SortDirection, SortOptions,
};
