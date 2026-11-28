//! Composition root: parse configuration, install the logger, create the database pool, start
//! lib-rustauth, wire every service on the database and security adapters, and build the app.
//! Every entrypoint starts here.

use std::sync::Arc;

use axum::Router;
use std::time::Duration;

use copilot_core::shared::constants::{SHUTDOWN_DRAIN_SECS, SHUTDOWN_GRACE_SECS};
use copilot_core::{
    Accounts, BoxError, ChatEngine, ChatRepositories, ChatService, Clock, FileService, LlmGateway,
    ModelService, ProjectService, ProviderService, ScheduleService, SecretCipher, SettingsService,
    SignInConfig, StdioPolicy, SystemClock, ToolAuth, ToolHost, ToolService,
};
use copilot_database::{DbPool, Env, PgStore};
use copilot_logger::LoggerOptions;
use copilot_mcp::{McpAuth, McpHost};
use copilot_providers::Gateway;
use copilot_security::{AesCipher, Purpose};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::config::{AppConfig, AppState};
use crate::helpers::auth::{Directory, Identity};
use crate::helpers::runs::RunRegistry;
use crate::helpers::scheduler::{Scheduler, settle};
use crate::helpers::slots::Slots;
use crate::helpers::worker::BlockingPool;
use crate::{App, AppDependencies, assemble};

pub struct ApiContext {
    pub config: AppConfig,
    pub app: Router,
    pub db: DbPool,
    /// The root token: cancelling it ends open streams, runs, and background tasks.
    pub shutdown: CancellationToken,
    /// The live runs, drained on shutdown.
    pub runs: Arc<RunRegistry>,
    /// The shared state, for the scheduled runs' records that shutdown waits on.
    state: AppState,
    purge: JoinHandle<()>,
    scheduler: JoinHandle<()>,
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
        let App { router, state } = assemble(AppDependencies {
            config: config.clone(),
            db: db.clone(),
            identity,
            adapters: Adapters::create(&config)?,
            shutdown: shutdown.clone(),
        });
        recover(&state).await?;
        // After recovery: a run it starts must not be marked interrupted.
        let scheduler = Scheduler::new(state.clone()).start(shutdown.clone());

        Ok(Self {
            config,
            app: router,
            db,
            shutdown,
            runs: Arc::clone(&state.runs),
            state,
            purge,
            scheduler,
        })
    }

    /// Stops the runs ([`drain`]) and the scheduler, waits for the scheduled runs to record how
    /// they ended, then releases resources. Call once the server has stopped accepting requests.
    pub async fn close(self) {
        drain(&self.runs).await;
        self.shutdown.cancel();
        for task in [self.scheduler, self.purge] {
            if let Err(error) = task.await {
                tracing::warn!(error = %error, "api.server.task.failed");
            }
        }
        if !settle(&self.state).await {
            tracing::warn!("api.scheduler.settle.incomplete");
        }
        copilot_database::close(&self.db).await;
    }
}

/// Marks the answers a stopped process left `streaming` as interrupted (`error`), before the
/// server takes requests. The next request of such a chat synthesizes results for their calls,
/// and Retry answers again.
pub async fn recover(state: &AppState) -> Result<u64, BoxError> {
    let recovered = state.chats.recover().await?;
    if recovered > 0 {
        tracing::info!(messages = recovered, "api.server.recovered");
    }
    Ok(recovered)
}

/// Shuts the run registry down: no new runs, `SHUTDOWN_GRACE_SECS` for live runs to finish,
/// then they are cancelled (saving their partial answers as `cancelled`) and get
/// `SHUTDOWN_DRAIN_SECS` more. Safe to call more than once: the times count from the first call.
pub async fn drain(runs: &RunRegistry) {
    let grace = Duration::from_secs(SHUTDOWN_GRACE_SECS);
    let limit = Duration::from_secs(SHUTDOWN_DRAIN_SECS);
    if !runs.drain(grace, limit).await {
        tracing::warn!("api.server.drain.incomplete");
    }
}

/// The outbound adapters the services call: model providers, MCP servers, and their sign-in
/// servers.
#[derive(Clone)]
pub struct Adapters {
    pub gateway: Arc<dyn LlmGateway>,
    pub host: Arc<dyn ToolHost>,
    pub auth: Arc<dyn ToolAuth>,
}

impl Adapters {
    /// The model providers through `copilot-providers` on the hardened provider clients
    /// (`copilot_security::clients(Purpose::Providers)`), and the MCP tool servers and their
    /// OAuth sign-in through `copilot-mcp` on the hardened tool clients
    /// (`clients(Purpose::Tools)`), with stdio servers' directories under `MCP_STDIO_DIR` when
    /// `MCP_STDIO_ENABLED` is on, which also makes this process non-dumpable
    /// (`copilot_mcp::harden`).
    pub fn create(config: &AppConfig) -> Result<Self, BoxError> {
        let providers = copilot_security::clients(Purpose::Providers)?;
        let tools = copilot_security::clients(Purpose::Tools)?;
        let mut host = McpHost::new(tools.clone());
        if config.stdio_enabled
            && let Some(dir) = &config.stdio_dir
        {
            // The programs run as this process's user: without this they could read its
            // environment (`SECRET_KEY`, the database password) from `/proc`.
            copilot_mcp::harden()?;
            host = host.with_stdio(dir.clone());
        }
        Ok(Self {
            gateway: Arc::new(Gateway::new(providers)),
            host: Arc::new(host),
            auth: Arc::new(McpAuth::new(tools)),
        })
    }

    /// Stand-ins for every adapter, for tests that never reach a provider or a tool server.
    #[cfg(test)]
    pub fn unavailable() -> Self {
        use crate::helpers::unavailable::Unavailable;

        Self {
            gateway: Arc::new(Unavailable),
            host: Arc::new(Unavailable),
            auth: Arc::new(Unavailable),
        }
    }
}

/// Where tool server sign-ins come back to: `<PUBLIC_URL>/api/oauth/callback`, and the Client
/// ID Metadata Document at `<PUBLIC_URL>/api/oauth/client` when `PUBLIC_URL` is `https` and
/// `OAUTH_CLIENT_METADATA` allows it. `None` without `PUBLIC_URL`: sign-in is off.
pub fn sign_in(config: &AppConfig) -> Option<SignInConfig> {
    let public = config.public_url.as_ref()?;
    let redirect_uri = public.join("/api/oauth/callback").ok()?;
    let client_metadata = (public.scheme() == "https" && config.client_metadata)
        .then(|| public.join("/api/oauth/client").ok())
        .flatten();
    Some(SignInConfig {
        redirect_uri,
        client_metadata,
    })
}

/// Builds every service once, on the database adapter ([`PgStore`]), the security adapter
/// ([`AesCipher`] under `SECRET_KEY`), the system clock, `adapters`, and `accounts` (lib-rustauth,
/// through `helpers::auth`).
pub fn wire(
    config: Arc<AppConfig>,
    db: DbPool,
    adapters: Adapters,
    identity: Directory,
    shutdown: CancellationToken,
) -> AppState {
    let accounts: Arc<dyn Accounts> = Arc::new(identity.clone());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let store = Arc::new(PgStore::new(db.clone(), clock.clone()));
    let stores = ChatRepositories::shared(store.clone());
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
    )
    .with_worker(Arc::new(BlockingPool));
    let mut tools = ToolService::new(
        stores.servers.clone(),
        adapters.host.clone(),
        cipher.clone(),
        clock.clone(),
        network,
    );
    if let Some(sign_in) = sign_in(&config) {
        tools = tools.with_sign_in(adapters.auth.clone(), store, sign_in);
    }
    // Before every start of a stdio server's process, its owner's account is read again.
    let tools = tools.with_stdio(StdioPolicy {
        enabled: config.stdio_enabled,
        accounts: accounts.clone(),
    });
    let schedules = ScheduleService::new(
        stores.schedules.clone(),
        stores.projects.clone(),
        stores.models.clone(),
        accounts,
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
        uploads: Arc::new(Slots::uploads()),
        downloads: Arc::new(Slots::downloads()),
        identity,
        tasks: TaskTracker::new(),
    }
}
