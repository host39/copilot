//! HTTP adapter: builds the axum app from its dependencies.
//!
//! Entrypoints live in `src/bin/` (`server` by default, `lambda` behind the `lambda` feature) and
//! get their dependencies from [`helpers::bootstrap::ApiContext::create`], the composition root.
//! Route handlers never wire services themselves.

pub mod config;
pub mod helpers;
pub mod routes;

use std::sync::Arc;

use axum::Router;
use axum::middleware::{from_fn, from_fn_with_state};
use copilot_core::{NoopTokenVerifier, RejectingTokenVerifier, TokenVerifier};
use copilot_database::DbPool;
use copilot_middleware::{CorsOptions, auth, cors, not_found, request_id, request_logger};

pub use crate::config::{AppConfig, AppState};

pub struct AppDependencies {
    pub config: AppConfig,
    pub db: DbPool,
    /// Overrides the verifier chosen from `AUTH_DISABLED`. Pass a real implementation here when
    /// the project adds authentication.
    pub token_verifier: Option<Arc<dyn TokenVerifier>>,
}

pub fn create_app(dependencies: AppDependencies) -> Router {
    let AppDependencies {
        config,
        db,
        token_verifier,
    } = dependencies;

    let verifier = token_verifier.unwrap_or_else(|| default_verifier(&config));
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
        .layer(from_fn_with_state(verifier, auth))
        .layer(from_fn(request_logger))
        .layer(from_fn(request_id))
        .layer(cors(cors_options))
        .with_state(state)
}

fn default_verifier(config: &AppConfig) -> Arc<dyn TokenVerifier> {
    if config.auth_disabled {
        Arc::new(NoopTokenVerifier)
    } else {
        Arc::new(RejectingTokenVerifier)
    }
}
