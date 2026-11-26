//! HTTP adapter: builds the axum app from its dependencies.
//!
//! The one entrypoint is `src/bin/server.rs`, a long-running process (Copilot runs exactly one API
//! process per install). It gets its dependencies from [`helpers::bootstrap::ApiContext::create`],
//! the composition root. Route handlers never wire services themselves.

pub mod config;
pub mod helpers;
pub mod routes;

use std::sync::Arc;

use axum::Router;
use axum::middleware::from_fn;
use copilot_database::DbPool;
use copilot_middleware::{CorsOptions, cors, not_found, request_id, request_logger};

pub use crate::config::{AppConfig, AppState};

pub struct AppDependencies {
    pub config: AppConfig,
    pub db: DbPool,
}

/// Builds the app. No route is protected yet: lib-rustauth's guard arrives with the first
/// protected routes (roadmap P1.7).
pub fn create_app(dependencies: AppDependencies) -> Router {
    let AppDependencies { config, db } = dependencies;

    let cors_options = CorsOptions {
        allow_localhost: config.environment.is_local(),
        allowed_origins: config.cors_allowed_origins.clone(),
    };
    let state = AppState {
        config: Arc::new(config),
        db,
    };

    let (router, openapi) = routes::router().split_for_parts();

    // `layer` wraps everything added before it, so the last layer is the outermost.
    router
        .merge(routes::docs::router(openapi))
        .fallback(not_found)
        .layer(from_fn(request_logger))
        .layer(from_fn(request_id))
        .layer(cors(cors_options))
        .with_state(state)
}
