//! Messages and their provider-neutral parts (`docs/design/copilot.md`, Stored JSON Shapes).

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::domains::chat::context::ContextHash;
use crate::domains::tool::ToolAlias;
use crate::shared::ids::{
    CallId, ChatId, FileId, MessageId, ModelId, ProviderId, ToolServerId, UserId,
};
use crate::shared::text::text_enum;

/// The error text `MessageRepository::recover` stores on messages that were streaming when the
/// server stopped.
pub const INTERRUPTED: &str = "Interrupted by a server restart.";
/// The reason given to pending tool calls when the user sends a new message instead of deciding.
pub const SKIPPED: &str = "Skipped by user.";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Role {
    User,
    Assistant,
    /// The results of one round of tool calls, in call order.
    Tool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MessageStatus {
    Streaming,
    Complete,
    AwaitingApproval,
    Cancelled,
    Error,
    Refused,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    Refusal,
    Error,
    Other,
    ToolLimit,
}

/// Where a tool call stands on approval.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Approval {
    /// Ran without asking.
    Auto,
    Pending,
    Approved,
    Denied,
}

/// How a tool call ended.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ToolStatus {
    Ok,
    Error,
    Denied,
    Cancelled,
    Timeout,
}

text_enum!(Role { User => "user", Assistant => "assistant", Tool => "tool" });
text_enum!(MessageStatus {
    Streaming => "streaming",
    Complete => "complete",
    AwaitingApproval => "awaiting_approval",
    Cancelled => "cancelled",
    Error => "error",
    Refused => "refused",
});
text_enum!(FinishReason {
    Stop => "stop",
    ToolCalls => "tool_calls",
    Length => "length",
    Refusal => "refusal",
    Error => "error",
    Other => "other",
    ToolLimit => "tool_limit",
});
text_enum!(Approval { Auto => "auto", Pending => "pending", Approved => "approved", Denied => "denied" });
text_enum!(ToolStatus {
    Ok => "ok",
    Error => "error",
    Denied => "denied",
    Cancelled => "cancelled",
    Timeout => "timeout",
});

/// Token counts of a provider request; every field optional, the last value wins.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Usage {
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub reasoning_tokens: Option<u32>,
    pub cache_read_tokens: Option<u32>,
    pub cache_write_tokens: Option<u32>,
}

impl Usage {
    /// Takes every value `later` reports; keeps the rest.
    pub fn merge(&mut self, later: Self) {
        let keep = |current: &mut Option<u32>, new: Option<u32>| {
            if new.is_some() {
                *current = new;
            }
        };
        keep(&mut self.input_tokens, later.input_tokens);
        keep(&mut self.output_tokens, later.output_tokens);
        keep(&mut self.reasoning_tokens, later.reasoning_tokens);
        keep(&mut self.cache_read_tokens, later.cache_read_tokens);
        keep(&mut self.cache_write_tokens, later.cache_write_tokens);
    }
}

/// A part of a message.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Text {
        text: String,
    },
    /// An attachment (a chat file).
    File {
        file_id: FileId,
        name: String,
        media_type: String,
    },
    Audio {
        file_id: FileId,
        format: String,
        transcript: Option<String>,
        duration_ms: Option<u32>,
    },
    /// Display only; never sent back except inside `provider_state`.
    Reasoning {
        text: String,
    },
    ToolCall(ToolCallPart),
    ToolResult(ToolResultPart),
}

/// A tool call the model made.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCallPart {
    pub id: CallId,
    pub provider_call_id: Option<String>,
    /// `None` for a built-in tool.
    pub server_id: Option<ToolServerId>,
    /// The original name, never the alias.
    pub tool_name: String,
    pub alias: ToolAlias,
    /// The arguments exactly as streamed.
    pub arguments_raw: String,
    /// The parsed arguments when they are a JSON object.
    pub arguments: Option<Value>,
    /// The arguments finished streaming.
    pub complete: bool,
    pub approval: Approval,
    pub deny_reason: Option<String>,
}

/// The result of one tool call.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolResultPart {
    pub call_id: CallId,
    pub status: ToolStatus,
    pub is_error: bool,
    pub content: Vec<ResultContent>,
    pub truncated: bool,
}

impl ToolResultPart {
    /// A result for a call that never ran: `denied` or `cancelled` with a one-line reason.
    pub fn skipped(call_id: CallId, status: ToolStatus, reason: &str) -> Self {
        Self {
            call_id,
            status,
            is_error: true,
            content: vec![ResultContent::Text(reason.to_owned())],
            truncated: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResultContent {
    Text(String),
    /// An image result, saved as a chat file.
    Image {
        file_id: FileId,
    },
}

/// A decision on a pending tool call.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Decision {
    Approve,
    Deny,
}

/// How far an approval reaches: this call, or this server's tool for the rest of the chat
/// (`chat_tool_approvals`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DecisionScope {
    #[default]
    Once,
    Chat,
}

/// One decision of `POST /api/chats/{id}/messages/{messageId}/decisions`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallDecision {
    pub call_id: CallId,
    pub decision: Decision,
    pub scope: DecisionScope,
    /// Passed to the model with a denial.
    pub reason: Option<String>,
}

/// A message (`messages`). Ordering comes from the stored `seq`; repositories return a chat's
/// messages in that order.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    id: MessageId,
    user_id: UserId,
    chat_id: ChatId,
    role: Role,
    status: MessageStatus,
    finish_reason: Option<FinishReason>,
    model_id: Option<ModelId>,
    provider_id: Option<ProviderId>,
    context_hash: Option<ContextHash>,
    parts: Vec<Part>,
    provider_state: Option<Value>,
    usage: Option<Usage>,
    error: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Every field of a stored message, for [`Message::restore`].
#[derive(Clone, Debug)]
pub struct MessageParts {
    pub id: MessageId,
    pub user_id: UserId,
    pub chat_id: ChatId,
    pub role: Role,
    pub status: MessageStatus,
    pub finish_reason: Option<FinishReason>,
    pub model_id: Option<ModelId>,
    pub provider_id: Option<ProviderId>,
    pub context_hash: Option<ContextHash>,
    pub parts: Vec<Part>,
    pub provider_state: Option<Value>,
    pub usage: Option<Usage>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Message {
    /// A complete user message.
    pub fn user(user_id: UserId, chat_id: ChatId, parts: Vec<Part>, now: DateTime<Utc>) -> Self {
        Self::new(
            user_id,
            chat_id,
            Role::User,
            MessageStatus::Complete,
            parts,
            now,
        )
    }

    /// A complete `tool` message with the results of one round, in call order.
    pub fn tool(
        user_id: UserId,
        chat_id: ChatId,
        results: Vec<ToolResultPart>,
        now: DateTime<Utc>,
    ) -> Self {
        let parts = results.into_iter().map(Part::ToolResult).collect();
        Self::new(
            user_id,
            chat_id,
            Role::Tool,
            MessageStatus::Complete,
            parts,
            now,
        )
    }

    /// An assistant message that starts streaming from `model` on `provider`.
    pub fn assistant(
        user_id: UserId,
        chat_id: ChatId,
        model: ModelId,
        provider: ProviderId,
        now: DateTime<Utc>,
    ) -> Self {
        let mut message = Self::new(
            user_id,
            chat_id,
            Role::Assistant,
            MessageStatus::Streaming,
            Vec::new(),
            now,
        );
        message.model_id = Some(model);
        message.provider_id = Some(provider);
        message
    }

    fn new(
        user_id: UserId,
        chat_id: ChatId,
        role: Role,
        status: MessageStatus,
        parts: Vec<Part>,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            id: MessageId::generate(),
            user_id,
            chat_id,
            role,
            status,
            finish_reason: None,
            model_id: None,
            provider_id: None,
            context_hash: None,
            parts,
            provider_state: None,
            usage: None,
            error: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// A row read back from storage.
    pub fn restore(parts: MessageParts) -> Self {
        Self {
            id: parts.id,
            user_id: parts.user_id,
            chat_id: parts.chat_id,
            role: parts.role,
            status: parts.status,
            finish_reason: parts.finish_reason,
            model_id: parts.model_id,
            provider_id: parts.provider_id,
            context_hash: parts.context_hash,
            parts: parts.parts,
            provider_state: parts.provider_state,
            usage: parts.usage,
            error: parts.error,
            created_at: parts.created_at,
            updated_at: parts.updated_at,
        }
    }

    /// Every field, for building a changed copy with [`Message::restore`].
    pub fn to_parts(&self) -> MessageParts {
        MessageParts {
            id: self.id,
            user_id: self.user_id,
            chat_id: self.chat_id,
            role: self.role,
            status: self.status,
            finish_reason: self.finish_reason,
            model_id: self.model_id,
            provider_id: self.provider_id,
            context_hash: self.context_hash,
            parts: self.parts.clone(),
            provider_state: self.provider_state.clone(),
            usage: self.usage,
            error: self.error.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }

    /// What `MessageRepository::decide` writes: each listed pending call becomes `approved` or
    /// `denied` (with its reason), and the message leaves `awaiting_approval` for `complete`.
    /// Calls that are not pending stay as they are.
    #[must_use]
    pub fn decided(mut self, decisions: &[CallDecision], now: DateTime<Utc>) -> Self {
        for part in &mut self.parts {
            let Part::ToolCall(call) = part else {
                continue;
            };
            if call.approval != Approval::Pending {
                continue;
            }
            if let Some(decision) = decisions
                .iter()
                .find(|decision| decision.call_id == call.id)
            {
                match decision.decision {
                    Decision::Approve => call.approval = Approval::Approved,
                    Decision::Deny => {
                        call.approval = Approval::Denied;
                        call.deny_reason.clone_from(&decision.reason);
                    }
                }
            }
        }
        self.status = MessageStatus::Complete;
        self.updated_at = now;
        self
    }

    /// The tool calls of this message, in call order.
    pub fn calls(&self) -> impl Iterator<Item = &ToolCallPart> {
        self.parts.iter().filter_map(|part| match part {
            Part::ToolCall(call) => Some(call),
            _ => None,
        })
    }

    /// The calls waiting for a decision.
    pub fn pending_calls(&self) -> impl Iterator<Item = &ToolCallPart> {
        self.calls()
            .filter(|call| call.approval == Approval::Pending)
    }

    /// The attachments of this message.
    pub fn files(&self) -> impl Iterator<Item = FileId> + '_ {
        self.parts.iter().filter_map(|part| match part {
            Part::File { file_id, .. } | Part::Audio { file_id, .. } => Some(*file_id),
            _ => None,
        })
    }

    /// The text parts joined by a line break (`text_content`, for search).
    pub fn text_content(&self) -> String {
        let texts: Vec<&str> = self
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        texts.join("\n")
    }

    pub fn id(&self) -> MessageId {
        self.id
    }

    pub fn user_id(&self) -> UserId {
        self.user_id
    }

    pub fn chat_id(&self) -> ChatId {
        self.chat_id
    }

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn status(&self) -> MessageStatus {
        self.status
    }

    pub fn finish_reason(&self) -> Option<FinishReason> {
        self.finish_reason
    }

    pub fn model_id(&self) -> Option<ModelId> {
        self.model_id
    }

    pub fn provider_id(&self) -> Option<ProviderId> {
        self.provider_id
    }

    pub fn context_hash(&self) -> Option<&ContextHash> {
        self.context_hash.as_ref()
    }

    pub fn parts(&self) -> &[Part] {
        &self.parts
    }

    /// Provider-opaque output; never sent to clients.
    pub fn provider_state(&self) -> Option<&Value> {
        self.provider_state.as_ref()
    }

    pub fn usage(&self) -> Option<&Usage> {
        self.usage.as_ref()
    }

    /// Human-readable.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

/// The results that close a paused round the user skipped by sending a new message: pending
/// calls are `denied` and calls that never ran are `cancelled`, both with "Skipped by user.".
pub fn skipped_results(paused: &Message) -> Vec<ToolResultPart> {
    paused
        .calls()
        .map(|call| {
            let status = match call.approval {
                Approval::Pending | Approval::Denied => ToolStatus::Denied,
                Approval::Auto | Approval::Approved => ToolStatus::Cancelled,
            };
            ToolResultPart::skipped(call.id, status, SKIPPED)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::domains::tool::Slug;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_767_225_600, 0).unwrap()
    }

    fn call(name: &str, approval: Approval) -> ToolCallPart {
        ToolCallPart {
            id: CallId::generate(),
            provider_call_id: Some(format!("call_{name}")),
            server_id: Some(ToolServerId::generate()),
            tool_name: name.to_owned(),
            alias: ToolAlias::new(&Slug::parse("gh").unwrap(), name),
            arguments_raw: "{}".to_owned(),
            arguments: Some(json!({})),
            complete: true,
            approval,
            deny_reason: None,
        }
    }

    fn paused(calls: Vec<ToolCallPart>) -> Message {
        let user = UserId::new(Uuid::now_v7());
        let message = Message::assistant(
            user,
            ChatId::generate(),
            ModelId::generate(),
            ProviderId::generate(),
            now(),
        );
        Message::restore(MessageParts {
            status: MessageStatus::AwaitingApproval,
            finish_reason: Some(FinishReason::ToolCalls),
            parts: calls.into_iter().map(Part::ToolCall).collect(),
            ..message.to_parts()
        })
    }

    #[test]
    fn builds_user_tool_and_assistant_messages() {
        let user = UserId::new(Uuid::now_v7());
        let chat = ChatId::generate();
        let message = Message::user(
            user,
            chat,
            vec![
                Part::Text {
                    text: "Hello".to_owned(),
                },
                Part::Text {
                    text: "there".to_owned(),
                },
            ],
            now(),
        );
        assert_eq!(message.role(), Role::User);
        assert_eq!(message.status(), MessageStatus::Complete);
        assert_eq!(message.text_content(), "Hello\nthere");
        let result = ToolResultPart::skipped(CallId::generate(), ToolStatus::Cancelled, "Stopped.");
        let tool = Message::tool(user, chat, vec![result.clone()], now());
        assert_eq!(tool.role(), Role::Tool);
        assert_eq!(tool.parts(), [Part::ToolResult(result)]);
        assert_eq!(tool.text_content(), "");
        let model = ModelId::generate();
        let assistant = Message::assistant(user, chat, model, ProviderId::generate(), now());
        assert_eq!(assistant.status(), MessageStatus::Streaming);
        assert_eq!(assistant.model_id(), Some(model));
    }

    #[test]
    fn applies_decisions_to_pending_calls_only() {
        let first = call("search", Approval::Pending);
        let second = call("delete", Approval::Pending);
        let auto = call("read", Approval::Auto);
        let message = paused(vec![first.clone(), second.clone(), auto.clone()]);
        assert_eq!(message.pending_calls().count(), 2);
        let decided = message.decided(
            &[
                CallDecision {
                    call_id: first.id,
                    decision: Decision::Approve,
                    scope: DecisionScope::Once,
                    reason: None,
                },
                CallDecision {
                    call_id: second.id,
                    decision: Decision::Deny,
                    scope: DecisionScope::Once,
                    reason: Some("Not now.".to_owned()),
                },
                CallDecision {
                    call_id: auto.id,
                    decision: Decision::Deny,
                    scope: DecisionScope::Once,
                    reason: None,
                },
            ],
            now(),
        );
        let approvals: Vec<(Approval, Option<&str>)> = decided
            .calls()
            .map(|call| (call.approval, call.deny_reason.as_deref()))
            .collect();
        assert_eq!(
            approvals,
            [
                (Approval::Approved, None),
                (Approval::Denied, Some("Not now.")),
                (Approval::Auto, None),
            ]
        );
        assert_eq!(decided.status(), MessageStatus::Complete);
        assert_eq!(decided.pending_calls().count(), 0);
    }

    #[test]
    fn skipping_denies_pending_calls_and_cancels_the_rest() {
        let pending = call("search", Approval::Pending);
        let auto = call("read", Approval::Auto);
        let results = skipped_results(&paused(vec![pending.clone(), auto.clone()]));
        assert_eq!(
            results,
            [
                ToolResultPart::skipped(pending.id, ToolStatus::Denied, SKIPPED),
                ToolResultPart::skipped(auto.id, ToolStatus::Cancelled, SKIPPED),
            ]
        );
        assert!(results.iter().all(|result| result.is_error));
    }

    #[test]
    fn merges_usage_with_the_last_value_winning() {
        let mut usage = Usage {
            input_tokens: Some(10),
            output_tokens: Some(1),
            ..Usage::default()
        };
        usage.merge(Usage {
            output_tokens: Some(5),
            cache_read_tokens: Some(3),
            ..Usage::default()
        });
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.output_tokens, Some(5));
        assert_eq!(usage.cache_read_tokens, Some(3));
    }

    #[test]
    fn lists_attachments() {
        let file = FileId::generate();
        let audio = FileId::generate();
        let message = Message::user(
            UserId::new(Uuid::now_v7()),
            ChatId::generate(),
            vec![
                Part::File {
                    file_id: file,
                    name: "a.png".to_owned(),
                    media_type: "image/png".to_owned(),
                },
                Part::Audio {
                    file_id: audio,
                    format: "wav".to_owned(),
                    transcript: None,
                    duration_ms: Some(1_000),
                },
            ],
            now(),
        );
        assert_eq!(message.files().collect::<Vec<_>>(), [file, audio]);
    }

    #[test]
    fn parses_stored_text_values() {
        assert_eq!(
            "awaiting_approval".parse::<MessageStatus>(),
            Ok(MessageStatus::AwaitingApproval)
        );
        assert_eq!(
            "tool_limit".parse::<FinishReason>(),
            Ok(FinishReason::ToolLimit)
        );
        assert_eq!(Role::Tool.as_str(), "tool");
        assert_eq!("timeout".parse::<ToolStatus>(), Ok(ToolStatus::Timeout));
        assert_eq!(Approval::Pending.to_string(), "pending");
        assert!("paused".parse::<MessageStatus>().is_err());
    }
}
