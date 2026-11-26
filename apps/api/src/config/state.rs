use std::sync::Arc;

use copilot_core::{
    ChatEngine, ChatService, FileService, ModelService, ProjectService, ProviderService,
    ScheduleService, SettingsService, ToolService,
};
use copilot_database::DbPool;
use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::helpers::runs::RunRegistry;

/// Shared state handed to every handler via `State<AppState>`. The composition root
/// (`helpers::bootstrap::wire`) builds every service once; handlers never build repositories or
/// services. Cheap to clone.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub db: DbPool,
    /// Cancelled when the server shuts down: streams end with it (`take_until`) and runs stop.
    pub shutdown: CancellationToken,
    pub settings: Arc<SettingsService>,
    pub providers: Arc<ProviderService>,
    pub models: Arc<ModelService>,
    pub projects: Arc<ProjectService>,
    pub files: Arc<FileService>,
    pub chats: Arc<ChatService>,
    pub tools: Arc<ToolService>,
    pub schedules: Arc<ScheduleService>,
    /// Runs turns: the model, tool calls, and approvals.
    pub engine: Arc<ChatEngine>,
    /// The chats that are answering now: one run per chat, a few per user.
    pub runs: Arc<RunRegistry>,
}
