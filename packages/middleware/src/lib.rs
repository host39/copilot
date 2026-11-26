//! HTTP cross-cutting concerns for axum apps.
//!
//! Layer order in `apps/api` (outermost first): [`cors()`], [`request_id()`], [`request_logger()`],
//! then [`auth()`] around the routes. Handlers return `Result<_, HttpError>` and let
//! [`HttpError`] map domain errors to status codes and the shared error body.
//!
//! This crate must not depend on `copilot-database`.

pub mod auth;
pub mod cors;
pub mod error_handler;
pub mod extract;
pub mod request_id;
pub mod request_logger;

pub use crate::auth::{PUBLIC_PATHS, UserId, auth};
pub use crate::cors::{CorsOptions, cors};
pub use crate::error_handler::{HttpError, not_found};
pub use crate::extract::JsonBody;
pub use crate::request_id::{REQUEST_ID_HEADER, RequestId, request_id};
pub use crate::request_logger::request_logger;
