//! Transport contracts: request and response DTOs shared by the API and the web app.
//!
//! - The API serializes these types and documents them in OpenAPI (`utoipa::ToSchema`).
//! - `cargo test -p copilot-contracts` exports TypeScript bindings to `ts/generated/`, which the
//!   Zod schemas in `ts/index.ts` must satisfy. Run `make contracts` after changing a type.
//! - Identity DTOs are lib-rustauth's: its router adds them to the OpenAPI document, and
//!   `ts/index.ts` re-exports `@rustauth/contracts`, so this crate never depends on rustauth.
//!
//! Wire rules:
//!
//! - JSON fields are camelCase; ids are UUID strings and timestamps RFC 3339 strings.
//! - Enum values are spelled as the spec writes them: statuses, reasons, and modes in
//!   snake_case (`awaiting_approval`, `tool_limit`), part kinds and capability names in
//!   camelCase, like the fields they name (`toolCall`, `audioIn`).
//! - Responses send every field; a missing value is `null`. Request fields with a default may
//!   be left out.
//! - `PATCH` bodies change only the fields they carry; `null` clears a nullable field.
//! - Secrets (API keys, bearer tokens, header values) only go in. Responses say whether one is
//!   stored ([`KeyState`]), and `Debug` on requests prints `***`.
//!
//! This crate must not depend on core, database, adapter, rustauth, or HTTP framework crates.

pub mod envelopes;
pub mod error;
pub mod file;
pub mod health;
pub mod model;
mod patch;
pub mod project;
pub mod provider;
pub mod schedule;
pub mod settings;
pub mod shared;
pub mod tool;

pub use crate::envelopes::{DetailsResponse, ListResponse};
pub use crate::error::ErrorResponse;
pub use crate::file::{
    Capacity, CapacityLevel, FileResponse, MoveFileRequest, UPLOAD_FIELD, UpdateFileRequest,
    UploadFileForm,
};
pub use crate::health::{HealthResponse, ReadyResponse, RootResponse};
pub use crate::model::{
    Capabilities, Capability, CapabilityLock, CapabilityOverrides, ModelResponse,
    UpdateModelRequest,
};
pub use crate::project::{
    CreateProjectRequest, ProjectDetailsResponse, ProjectListQuery, ProjectResponse,
    UpdateProjectRequest,
};
pub use crate::provider::{
    CreateProviderRequest, Preset, ProviderDetailsResponse, ProviderKind, ProviderResponse,
    ProviderUsage, RefreshResponse, UpdateProviderRequest,
};
pub use crate::schedule::{
    CreateScheduleRequest, CreatedBy, OnAsk, RunStatus, ScheduleListQuery, SchedulePreviewRequest,
    SchedulePreviewResponse, ScheduleResponse, ScheduleRunResponse, UpdateScheduleRequest,
};
pub use crate::settings::{SettingsResponse, UpdateSettingsRequest};
pub use crate::shared::{BuiltinTool, KeyState, ReasoningLevel};
pub use crate::tool::{
    ApprovalMode, CreateToolServerRequest, Credentials, ServerTool, ToolChanges, ToolMode,
    ToolServerResponse, ToolTestResponse, UpdateToolServerRequest,
};
