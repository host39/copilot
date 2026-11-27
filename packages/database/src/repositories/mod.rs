//! `sqlx` implementations of the repository traits (ports) defined in `copilot-core`.
//!
//! [`PgStore`] is the database adapter: one store over the pool that implements every repository
//! port, so the composition root wires it once (`ChatRepositories::shared`). Each port has its
//! own module, and `jsonb` columns go through the versioned records in [`crate::records`].
//!
//! Every port method is implemented. Timestamps a repository sets itself come from the `Clock`
//! given to [`PgStore::new`]. Domain rules stay in core: where a write depends on the row's
//! current state (a decision, a run record), the repository locks the row, applies the core
//! entity's method, and writes the result. Map driver errors with
//! `store::failed("database.<table>.<operation>.failed")` and unreadable stored values with
//! `store::invalid("database.<table>.<column>.invalid")`.

mod chats;
mod files;
mod projects;
mod providers;
mod schedules;
mod settings;
pub mod store;
mod tools;

pub use crate::repositories::store::PgStore;
