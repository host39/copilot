//! `tool`: MCP servers, remote or stdio (slugs, aliases, tool modes, cached tool lists,
//! changed-tool detection, OAuth sign-in, and the stdio rules) and the server tools a turn gets.

mod auth;
mod entities;
mod headers;
mod ports;
mod selection;
mod service;
mod stdio;
mod values;

pub use crate::domains::tool::auth::{
    AuthMode, ClientConfig, Pending, Session, SignIn, SignInFlow, SignInState,
};
pub use crate::domains::tool::entities::{Location, ToolCache, ToolServer, ToolServerParts};
pub use crate::domains::tool::headers::ToolHeaders;
pub use crate::domains::tool::ports::{SignInRepository, ToolServerRepository};
pub use crate::domains::tool::selection::{
    EffectiveTool, EffectiveTools, ServerOverride, ToolChanges, ToolSelection, ToolSpec,
    UnavailableServer, offered,
};
pub use crate::domains::tool::service::{
    CLIENT_ID_REQUIRED, CLIENT_NAME, Callback, ClientChange, ClientInput, CredentialChange,
    NewToolServer, SIGN_IN_UNAVAILABLE, SignInConfig, SignInOutcome, SignInResult, SignInStart,
    ToolServerDetails, ToolServerUpdate, ToolService, ToolTest,
};
pub use crate::domains::tool::stdio::{
    STDIO_DISABLED, StdioCommand, StdioEnv, StdioInput, StdioPolicy, StdioUpdate, Transport,
};
pub use crate::domains::tool::values::{
    ALIAS_MAX_CHARS, ApprovalMode, SLUG_MAX_CHARS, Slug, ToolAlias, ToolMode,
};
