//! Composition root: parse configuration, install the logger, create the database pool, start
//! lib-rustauth, wire every service on the database and security adapters, and build the app.
//! Every entrypoint starts here.

use std::sync::Arc;

use axum::Router;
use copilot_core::{
    BoxError, ChatEngine, ChatRepositories, ChatService, Clock, FileService, LlmGateway,
    ModelService, ProjectService, ProviderService, ScheduleService, SecretCipher, SettingsService,
    SystemClock, ToolHost, ToolService,
};
use copilot_database::{DbPool, Env, PgStore};
use copilot_logger::LoggerOptions;
use copilot_mcp::McpHost;
use copilot_providers::Gateway;
use copilot_security::{AesCipher, Purpose};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config::{AppConfig, AppState};
use crate::helpers::auth::Identity;
use crate::helpers::runs::RunRegistry;
use crate::helpers::uploads::UploadSlots;
use crate::{AppDependencies, create_app};

pub struct ApiContext {
    pub config: AppConfig,
    pub app: Router,
    pub db: DbPool,
    /// The root token: cancelling it ends open streams, runs, and background tasks.
    pub shutdown: CancellationToken,
    purge: JoinHandle<()>,
}

impl ApiContext {
    /// Builds the context from the process environment. Must run inside a Tokio runtime, and
    /// needs the database: lib-rustauth checks its `auth` schema at startup (run `make migrate`
    /// first).
    pub async fn create() -> Result<Self, BoxError> {
        let config = AppConfig::parse(&Env::load())?;
        copilot_logger::init(LoggerOptions {
            environment: config.environment,
            level: config.log_level,
        })?;

        let db = copilot_database::create_pool(&config.db);
        let identity = Identity::start(db.clone(), &config.identity).await?;
        let shutdown = CancellationToken::new();
        let purge = identity.purge(shutdown.clone());
        let app = create_app(AppDependencies {
            config: config.clone(),
            db: db.clone(),
            identity,
            adapters: Adapters::create()?,
            shutdown: shutdown.clone(),
        });

        Ok(Self {
            config,
            app,
            db,
            shutdown,
            purge,
        })
    }

    /// Releases resources. Call once the server has stopped accepting requests.
    pub async fn close(self) {
        self.shutdown.cancel();
        if let Err(error) = self.purge.await {
            tracing::warn!(error = %error, "api.server.task.failed");
        }
        copilot_database::close(&self.db).await;
    }
}

/// The outbound adapters the services call: model providers and MCP servers.
#[derive(Clone)]
pub struct Adapters {
    pub gateway: Arc<dyn LlmGateway>,
    pub host: Arc<dyn ToolHost>,
}

impl Adapters {
    /// The model providers through `copilot-providers` on the hardened provider clients
    /// (`copilot_security::clients(Purpose::Providers)`), and the MCP tool servers through
    /// `copilot-mcp` on the hardened tool clients (`clients(Purpose::Tools)`).
    pub fn create() -> Result<Self, BoxError> {
        let providers = copilot_security::clients(Purpose::Providers)?;
        let tools = copilot_security::clients(Purpose::Tools)?;
        Ok(Self {
            gateway: Arc::new(Gateway::new(providers)),
            host: Arc::new(McpHost::new(tools)),
        })
    }

    /// Stand-ins for both adapters, for tests that never reach a provider or a tool server.
    #[cfg(test)]
    pub fn unavailable() -> Self {
        use crate::helpers::unavailable::Unavailable;

        Self {
            gateway: Arc::new(Unavailable),
            host: Arc::new(Unavailable),
        }
    }
}

/// Builds every service once, on the database adapter ([`PgStore`]), the security adapter
/// ([`AesCipher`] under `SECRET_KEY`), the system clock, and `adapters`.
pub fn wire(
    config: Arc<AppConfig>,
    db: DbPool,
    adapters: Adapters,
    shutdown: CancellationToken,
) -> AppState {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let stores = ChatRepositories::shared(Arc::new(PgStore::new(db.clone(), clock.clone())));
    let cipher: Arc<dyn SecretCipher> = Arc::new(AesCipher::new(config.secret_key.clone()));
    let network = config.private_network;

    let settings = SettingsService::new(stores.settings.clone(), clock.clone());
    let providers = ProviderService::new(
        stores.providers.clone(),
        stores.models.clone(),
        stores.schedules.clone(),
        adapters.gateway.clone(),
        cipher.clone(),
        clock.clone(),
        network,
    );
    let models = ModelService::new(
        stores.models.clone(),
        stores.providers.clone(),
        stores.schedules.clone(),
        clock.clone(),
    );
    let projects = ProjectService::new(
        stores.projects.clone(),
        stores.models.clone(),
        stores.servers.clone(),
        clock.clone(),
    );
    let files = FileService::new(
        stores.files.clone(),
        stores.projects.clone(),
        stores.chats.clone(),
        stores.models.clone(),
        clock.clone(),
        config.files,
    );
    let tools = ToolService::new(
        stores.servers.clone(),
        adapters.host.clone(),
        cipher.clone(),
        clock.clone(),
        network,
    );
    let schedules = ScheduleService::new(
        stores.schedules.clone(),
        stores.projects.clone(),
        stores.models.clone(),
        clock.clone(),
    );
    let engine = ChatEngine::new(
        adapters.gateway,
        adapters.host,
        stores.messages.clone(),
        tools.clone(),
        schedules.clone(),
        files.clone(),
        clock.clone(),
    );
    let chats = ChatService::new(stores, tools.clone(), cipher, clock, network);
    let runs = RunRegistry::new(shutdown.clone());

    AppState {
        config,
        db,
        shutdown,
        settings: Arc::new(settings),
        providers: Arc::new(providers),
        models: Arc::new(models),
        projects: Arc::new(projects),
        files: Arc::new(files),
        chats: Arc::new(chats),
        tools: Arc::new(tools),
        schedules: Arc::new(schedules),
        engine: Arc::new(engine),
        runs: Arc::new(runs),
        uploads: Arc::new(UploadSlots::default()),
    }
}
