//! Route modules. Each exposes `router()` with its OpenAPI-documented handlers; mount it here.

pub mod docs;
pub mod health;

use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

use crate::config::AppState;
use crate::routes::docs::ApiDoc;

/// Every documented route, collected into one router plus the OpenAPI document.
pub fn router() -> OpenApiRouter<AppState> {
    // utoipa fills `info.license` from the workspace `license` in Cargo.toml.
    OpenApiRouter::with_openapi(ApiDoc::openapi()).merge(health::router())
}
