//! `sqlx` implementations of the repository traits (ports) defined in `copilot-core`.
//!
//! [`PgStore`] is the database adapter: one store over the pool that implements every repository
//! port, so the composition root wires it once (`ChatRepositories::shared`). Each port has its
//! own module. Until roadmap step P2.A fills a module in, its methods fail with
//! `database.repository.unimplemented` (see [`store::pending`]); the API routes that would call
//! them answer `501` meanwhile. Map driver errors with
//! `DomainError::internal("database.<area>.<operation>.failed", error)`.

mod chats;
mod files;
mod projects;
mod providers;
mod schedules;
mod settings;
pub mod store;
mod tools;

pub use crate::repositories::store::PgStore;
