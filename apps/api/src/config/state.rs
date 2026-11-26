use std::sync::Arc;

use copilot_database::DbPool;

use crate::config::AppConfig;

/// Shared state handed to every handler via `State<AppState>`.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub db: DbPool,
}
