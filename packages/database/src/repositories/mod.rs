//! `sqlx` implementations of the repository traits (ports) defined in `copilot-core`.
//!
//! [`PgStore`] is the database adapter: one store over the pool that implements every repository
//! port, so the composition root wires it once (`ChatRepositories::shared`). Each port has its
//! own module, and `jsonb` columns go through the versioned records in [`crate::records`].
//!
//! The walking skeleton (P1.9) wrote what a turn needs, and roadmap step P2.A wrote every other
//! domain. Timestamps a repository sets itself come from the `Clock` given to [`PgStore::new`].
//! Two methods still fail with `database.repository.unimplemented` (see [`store::pending`]) until
//! P3.1 writes them: `MessageRepository::decide` (the decisions route) and
//! `MessageRepository::recover` (startup recovery); the routes that would call them answer `501`
//! meanwhile. Map driver errors with `store::failed("database.<table>.<operation>.failed")` and
//! unreadable stored values with `store::invalid("database.<table>.<column>.invalid")`.

mod chats;
mod files;
mod projects;
mod providers;
mod schedules;
mod settings;
pub mod store;
mod tools;

pub use crate::repositories::store::PgStore;
