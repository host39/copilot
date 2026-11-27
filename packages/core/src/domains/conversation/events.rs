//! What a run reports while it happens (`docs/design/copilot.md`, Stream Events).

use bytes::Bytes;
use serde_json::Value;

use crate::domains::chat::{FinishReason, MessageStatus, ResultContent, Role, ToolStatus, Usage};
use crate::ports::llm_gateway::{PartKind, Refusal};
use crate::shared::ids::{CallId, MessageId, ToolServerId};

/// One event of a run. `contracts` maps it to `ChatStreamEvent`; the API adds `seq` and
/// truncates tool output for display.
#[derive(Clone, Debug, PartialEq)]
pub enum TurnEvent {
    MessageStarted {
        message_id: MessageId,
        role: Role,
        /// The first message in the context window; `None` when the window starts at the first
        /// message of the chat.
        window_start_id: Option<MessageId>,
        /// The names of the selected tool servers whose tools this answer goes without (they
        /// could not be listed); set on a turn's first answer only.
        unavailable: Vec<String>,
    },
    /// `index` is the part's position in the stored message.
    PartStarted {
        message_id: MessageId,
        index: u32,
        kind: PartKind,
    },
    TextDelta {
        message_id: MessageId,
        index: u32,
        text: String,
    },
    ReasoningDelta {
        message_id: MessageId,
        index: u32,
        text: String,
    },
    AudioDelta {
        message_id: MessageId,
        data: Bytes,
        format: String,
    },
    /// A tool call of an assistant message, once its arguments are complete.
    ToolRequested {
        message_id: MessageId,
        call_id: CallId,
        /// `None` for a built-in tool.
        server_id: Option<ToolServerId>,
        /// The original name, never the alias.
        tool_name: String,
        /// The tool's display title, else its name.
        title: String,
        /// The parsed arguments, or the raw text when they are not a JSON object.
        arguments: Value,
        needs_approval: bool,
    },
    /// A result in the `tool` message `message_id`.
    ToolFinished {
        message_id: MessageId,
        call_id: CallId,
        status: ToolStatus,
        /// As stored (after core's `MCP_RESULT_MAX_BYTES` cut).
        output: Vec<ResultContent>,
        truncated: bool,
    },
    MessageFinished {
        message_id: MessageId,
        status: MessageStatus,
        finish_reason: Option<FinishReason>,
        refusal: Option<Refusal>,
        usage: Option<Usage>,
    },
    /// The run failed; `message` is human-readable.
    Error { message: String, code: String },
    /// The last event of every run.
    Done,
}
