//! What `ChatService::prepare` hands to the conversation engine.

use std::collections::HashMap;

use crate::domains::chat::context::TurnContext;
use crate::domains::chat::entities::Chat;
use crate::domains::chat::message::{CallDecision, Message};
use crate::domains::file::StoredFile;
use crate::domains::project::Project;
use crate::domains::provider::{Capabilities, Model, Provider};
use crate::domains::schedule::{OnAsk, Schedule};
use crate::domains::settings::UserSettings;
use crate::domains::tool::{EffectiveTools, ToolServer};
use crate::ports::llm_gateway::ModelTarget;
use crate::shared::actor::Actor;
use crate::shared::ids::{FileId, MessageId};

/// What the user asked for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TurnInput {
    /// A new message, or an edit of `edit_from` (that message and every later one are replaced).
    Send {
        text: String,
        file_ids: Vec<FileId>,
        edit_from: Option<MessageId>,
    },
    /// Answer the last user message again (also Retry on an error banner).
    Regenerate,
    /// Decisions on the pending tool calls of `message_id`; the run continues from its snapshot.
    Decide {
        message_id: MessageId,
        decisions: Vec<CallDecision>,
    },
}

/// Whether someone is there to answer approvals.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunMode {
    Interactive,
    /// A schedule run: `ask` tools follow `on_ask`, and schedule tools are never offered.
    Unattended {
        on_ask: OnAsk,
    },
}

/// A validated turn, with the user message (or tail change) and the turn's snapshot already
/// saved. `ChatEngine::run` builds every request of the turn from `context`.
#[derive(Clone, Debug)]
pub struct PreparedTurn {
    pub actor: Actor,
    pub chat: Chat,
    pub mode: RunMode,
    pub settings: UserSettings,
    pub project: Option<Project>,
    /// The schedule of a run chat; a standalone schedule's instructions replace the chat's.
    pub schedule: Option<Schedule>,
    pub model: Model,
    pub provider: Provider,
    /// After overrides and locks.
    pub capabilities: Capabilities,
    /// With the opened key.
    pub target: ModelTarget,
    /// The project's included files, oldest first.
    pub files: Vec<StoredFile>,
    /// Metadata of every file the snapshot or the history names (the preamble, attachments,
    /// tool result images). A named file that is missing here was deleted.
    pub references: HashMap<FileId, StoredFile>,
    /// The server tools this turn listed, with the servers that could not be listed (the UI
    /// names them). Empty for a decision, which keeps its snapshot's tools.
    pub tools: EffectiveTools,
    /// The current settings of the servers the snapshot offers tools from, for the approval
    /// policy and the tool session.
    pub servers: Vec<ToolServer>,
    /// The chat's messages after the write, ordered.
    pub history: Vec<Message>,
    /// The turn's snapshot: built for this turn, or the paused turn's for a decision.
    pub context: TurnContext,
}

impl PreparedTurn {
    /// The message the run answers or continues: the last one in the history.
    pub fn last_message(&self) -> Option<&Message> {
        self.history.last()
    }
}
