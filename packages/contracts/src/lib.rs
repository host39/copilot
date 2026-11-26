//! Transport contracts: request and response DTOs shared by the API and the web app.
//!
//! - The API serializes these types and documents them in OpenAPI (`utoipa::ToSchema`).
//! - `cargo test -p copilot-contracts` exports TypeScript bindings to `ts/generated/`, which the
//!   Zod schemas in `ts/index.ts` must satisfy. Run `make contracts` after changing a type.
//!
//! JSON fields are camelCase on the wire. This crate must not depend on core, database, or
//! HTTP framework crates.

pub mod envelopes;
pub mod error;
pub mod health;

pub use crate::envelopes::{DetailsResponse, ListResponse};
pub use crate::error::ErrorResponse;
pub use crate::health::{HealthResponse, ReadyResponse, RootResponse};
