//! Fakes for every core port, for unit tests in core and in other crates.
//!
//! Compiled for core's own tests and, in other crates, through the `testing` feature
//! (`copilot-core = { workspace = true, features = ["testing"] }` under `[dev-dependencies]`).
//! Never enable the feature for a normal dependency.

mod cipher;
mod clock;
mod file;
mod gateway;
mod project;
mod provider;
mod settings;
mod store;

use uuid::Uuid;

pub use crate::testing::cipher::FakeCipher;
pub use crate::testing::clock::FixedClock;
pub use crate::testing::gateway::{ScriptedGateway, remote_model};
pub use crate::testing::store::MemoryStore;

use crate::shared::actor::Actor;
use crate::shared::ids::UserId;

/// A plain user with a fresh id.
pub fn actor() -> Actor {
    Actor::user(UserId::new(Uuid::now_v7()))
}

/// An administrator with a fresh id.
pub fn admin() -> Actor {
    Actor::admin(UserId::new(Uuid::now_v7()))
}
