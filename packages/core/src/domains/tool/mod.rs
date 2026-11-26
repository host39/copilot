//! `tool`: remote MCP servers (slugs, aliases, tool modes, cached tool lists, and changed-tool
//! detection) and the server tools a turn gets.

mod entities;
mod headers;
mod ports;
mod selection;
mod service;
mod values;

pub use crate::domains::tool::entities::{ToolCache, ToolServer, ToolServerParts};
pub use crate::domains::tool::headers::ToolHeaders;
pub use crate::domains::tool::ports::ToolServerRepository;
pub use crate::domains::tool::selection::{
    EffectiveTool, EffectiveTools, ServerOverride, ToolChanges, ToolSelection, ToolSpec, offered,
};
pub use crate::domains::tool::service::{
    CredentialChange, NewToolServer, ToolServerDetails, ToolServerUpdate, ToolService, ToolTest,
};
pub use crate::domains::tool::values::{
    ALIAS_MAX_CHARS, ApprovalMode, SLUG_MAX_CHARS, Slug, ToolAlias, ToolMode,
};
