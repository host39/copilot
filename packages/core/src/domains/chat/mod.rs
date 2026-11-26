//! `chat`: chats, their messages and turn snapshots, read state, search, save as project, and
//! turn preparation (`prepare`, which saves the user message and hands the engine a
//! `PreparedTurn`).

mod context;
mod entities;
mod message;
mod ports;
mod service;
mod turn;

pub use crate::domains::chat::context::{ContextHash, SnapshotFile, SnapshotTool, TurnContext};
pub use crate::domains::chat::entities::{Chat, ChatParts};
pub use crate::domains::chat::message::{
    Approval, CallDecision, Decision, DecisionScope, FinishReason, INTERRUPTED, Message,
    MessageParts, MessageStatus, Part, ResultContent, Role, SKIPPED, ToolCallPart, ToolResultPart,
    ToolStatus, Usage, skipped_results,
};
pub use crate::domains::chat::ports::{
    ChatCursor, ChatQuery, ChatRepository, ChatSummary, MessageRepository, Page, SearchHit,
    TurnContextRepository,
};
pub use crate::domains::chat::service::{ChatRepositories, ChatService, ChatUpdate, NewChat};
pub use crate::domains::chat::turn::{PreparedTurn, RunMode, TurnInput};
