//! Domain layer: entities, services, repository ports, and shared errors.
//!
//! Business domains live under `src/domains/<name>/` and are re-exported here. See
//! `docs/adr/001-domain-driven-design.md` for the layout and `docs/design/copilot.md` (Domain
//! Model) for the rules.
//!
//! This crate must not depend on HTTP, database, crypto, logging, or identity implementations.
//! Callers arrive as an [`Actor`] that `apps/api` builds from lib-rustauth's user.

pub mod domains;
pub mod ports;
pub mod shared;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

// Domains
pub use crate::domains::chat::{
    Approval, ApprovedTool, CallDecision, Chat, ChatCursor, ChatParts, ChatQuery, ChatRepositories,
    ChatRepository, ChatService, ChatSummary, ChatUpdate, ContextHash, Decision, DecisionScope,
    FinishReason, INTERRUPTED, MODEL_UNAVAILABLE, Message, MessageParts, MessageRepository,
    MessageStatus, NewChat, Page, Part, PreparedTurn, Rendering, ResultContent, Role, RunMode,
    SKIPPED, ScheduledTurn, SearchHit, SnapshotFile, SnapshotTool, ToolCallPart, ToolResultPart,
    ToolStatus, TurnContext, TurnContextRepository, TurnInput, Usage, skipped_results,
};
pub use crate::domains::conversation::{ChatEngine, ContextInputs, Draft, TurnEvent, TurnOutcome};
pub use crate::domains::file::{
    Capacity, CapacityLevel, FileKind, FileLimits, FileOwner, FileRepository, FileService,
    FileUpdate, Measured, MediaType, OCTET_STREAM, StoredFile, StoredFileParts, served_type,
};
pub use crate::domains::project::{
    NewProject, Project, ProjectParts, ProjectQuery, ProjectRepository, ProjectService,
    ProjectUpdate,
};
pub use crate::domains::provider::{
    ANTHROPIC_VERSION, Capabilities, Capability, CapabilityOverrides, DetectedModel, Dialect,
    KeyChange, KeyState, Lock, Locks, Model, ModelParts, ModelQuery, ModelRepository, ModelService,
    ModelUpdate, ModelView, NewProvider, Preset, Provider, ProviderDetails, ProviderKind,
    ProviderParts, ProviderRepository, ProviderService, ProviderUpdate, ProviderUsage, SyncReport,
    reasoning_off_with_tools,
};
pub use crate::domains::schedule::{
    CreatedBy, CronRule, Firing, MODEL_REMOVED, MODEL_TURNED_OFF, NewSchedule, OnAsk, RUN_FAILED,
    RUN_TIMED_OUT, RunOutcome, RunRecord, RunStatus, START_FAILED, Schedule, ScheduleParts,
    SchedulePreview, SchedulePrompt, ScheduleQuery, ScheduleRepository, ScheduleService,
    ScheduleTool, ScheduleUpdate, ToolOrigin, cancel_target,
};
pub use crate::domains::settings::{SettingsRepository, SettingsService, UserSettings};
pub use crate::domains::tool::{
    ALIAS_MAX_CHARS, ApprovalMode, CredentialChange, EffectiveTool, EffectiveTools, NewToolServer,
    SLUG_MAX_CHARS, ServerOverride, Slug, ToolAlias, ToolCache, ToolChanges, ToolHeaders, ToolMode,
    ToolSelection, ToolServer, ToolServerDetails, ToolServerParts, ToolServerRepository,
    ToolServerUpdate, ToolService, ToolSpec, ToolTest, offered,
};
// Ports (interfaces implemented by adapters in other crates)
pub use crate::ports::clock::{Clock, SystemClock};
pub use crate::ports::llm_gateway::{
    AudioOut, LlmEvent, LlmGateway, LlmMessage, LlmPart, LlmRequest, ModelTarget, PartKind,
    ProviderTarget, Refusal, RemoteModel, ReportedCapabilities,
};
pub use crate::ports::logger::LogLevel;
pub use crate::ports::secret_cipher::{SECRET_UNREADABLE, SealContext, SecretCipher, unreadable};
pub use crate::ports::tool_host::{
    RemoteTool, TOOL_TIMEOUT, TOOL_UNKNOWN, ToolContent, ToolEndpoint, ToolHost, ToolListing,
    ToolOutput, ToolSession,
};
pub use crate::ports::turn_sink::TurnSink;
// Shared types
pub use crate::shared::actor::Actor;
pub use crate::shared::base_url::BaseUrl;
pub use crate::shared::builtin_tool::BuiltinTool;
pub use crate::shared::caps::{CapError, cap_events, read_capped};
pub use crate::shared::constants::{DEFAULT_PAGE_OFFSET, DEFAULT_PAGE_SIZE};
pub use crate::shared::errors::{BoxError, Context, DomainError, DomainResult};
pub use crate::shared::ids::{
    CallId, ChatId, FileId, MessageId, ModelId, ProjectId, ProviderId, ScheduleId, ToolServerId,
    UserId,
};
pub use crate::shared::net::{
    AddressClass, Blocked, NetworkScope, Outbound, PrivateNetwork, classify, is_allowed,
    is_allowed_plaintext,
};
pub use crate::shared::reasoning::ReasoningLevel;
pub use crate::shared::secret::Secret;
pub use crate::shared::types::{
    Environment, PaginatedResult, PaginationOptions, SortDirection, SortOptions,
};
