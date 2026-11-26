//! HTTP adapter: builds the axum app from its dependencies.
//!
//! The one entrypoint is `src/bin/server.rs`, a long-running process (Copilot runs exactly one API
//! process per install). It gets its dependencies from [`helpers::bootstrap::ApiContext::create`],
//! the composition root. Route handlers never wire services themselves.
//!
//! Routing: `/health` and `/health/ready` at the root for probes; every other route under `/api`
//! (lib-rustauth's `/api/auth/*` and `/api/admin/users*`, Copilot's routes behind its guard, and
//! the public `/api/doc` and `/api/ui`); with `WEB_DIR` set, the web bundle for any other `GET`.

pub mod config;
pub mod helpers;
pub mod routes;

use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::middleware::from_fn;
use copilot_database::DbPool;
use copilot_middleware::{CorsOptions, cors, not_found, request_id, request_logger};
use tokio_util::sync::CancellationToken;

pub use crate::config::{AppConfig, AppState};
use crate::helpers::auth::Identity;
use crate::helpers::bootstrap::{Adapters, wire};

pub struct AppDependencies {
    pub config: AppConfig,
    pub db: DbPool,
    pub identity: Identity,
    pub adapters: Adapters,
    pub shutdown: CancellationToken,
}

/// Builds the app: every service wired once, rustauth's routes and guard, Copilot's routes, the
/// OpenAPI document, and the web bundle.
pub fn create_app(dependencies: AppDependencies) -> Router {
    let AppDependencies {
        config,
        db,
        identity,
        adapters,
        shutdown,
    } = dependencies;

    let cors_options = CorsOptions {
        allow_localhost: config.environment.is_local(),
        allowed_origins: config.cors_allowed_origins.clone(),
    };
    let web_dir = config.web_dir.clone();
    let state = wire(Arc::new(config), db, adapters, shutdown);

    let (router, openapi) = routes::router(&identity).split_for_parts();
    let router = router.merge(routes::docs::router(openapi));
    let router = match web_dir {
        Some(dir) => {
            let files = routes::web::files(&dir);
            router.fallback(move |request: Request| routes::web::serve(files.clone(), request))
        }
        None => router.fallback(not_found),
    };

    // `layer` wraps everything added before it, so the last layer is the outermost. The request
    // id runs outside rustauth's router and guard, so their error bodies echo it too.
    router
        .layer(from_fn(request_logger))
        .layer(from_fn(request_id))
        .layer(cors(cors_options))
        .with_state(state)
}
