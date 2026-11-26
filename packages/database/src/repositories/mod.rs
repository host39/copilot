//! `sqlx` implementations of the repository traits (ports) defined in `copilot-core`.
//!
//! [`PgStore`] is the database adapter: one store over the pool that implements every repository
//! port, so the composition root wires it once (`ChatRepositories::shared`). Each port has its
//! own module, and `jsonb` columns go through the versioned records in [`crate::records`].
//!
//! The walking skeleton (P1.9) wrote what a turn needs: settings, connections and models, chats
//! with their join rows, message history and writes, turn snapshots, and reading tool servers,
//! plus what its routes reach on the way (`ScheduleRepository::disable_for_model` for a model
//! turned off). Roadmap step P2.A writes the rest by domain: so far the provider usage counts,
//! projects with their tool server rows, and files with their bytes. Timestamps a repository
//! sets itself come from the `Clock` given to [`PgStore::new`].
//! Every other method fails with `database.repository.unimplemented` (see [`store::pending`])
//! until P2.A (or P3.1) writes it; the API routes that would call them answer `501`
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
