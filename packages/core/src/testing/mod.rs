//! Fakes for every core port, for unit tests in core and in other crates.
//!
//! Compiled for core's own tests and, in other crates, through the `testing` feature
//! (`copilot-core = { workspace = true, features = ["testing"] }` under `[dev-dependencies]`).
//! Never enable the feature for a normal dependency.

mod chat;
mod cipher;
mod clock;
mod file;
mod gateway;
mod project;
mod provider;
mod schedule;
mod settings;
mod sink;
mod store;
mod tool;
mod tool_host;

use uuid::Uuid;

pub use crate::testing::cipher::FakeCipher;
pub use crate::testing::clock::FixedClock;
pub use crate::testing::gateway::{ScriptedGateway, Step, call_reply, remote_model, text_reply};
pub use crate::testing::sink::RecordingSink;
pub use crate::testing::store::MemoryStore;
pub use crate::testing::tool_host::{CallStep, RecordedCall, ScriptedToolHost, remote_tool};

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
