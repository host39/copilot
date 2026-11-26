//! Chat, message, and turn snapshot persistence ports.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domains::chat::context::{ContextHash, TurnContext};
use crate::domains::chat::entities::Chat;
use crate::domains::chat::message::{CallDecision, Message};
use crate::shared::constants::DEFAULT_PAGE_SIZE;
use crate::shared::errors::DomainResult;
use crate::shared::ids::{ChatId, MessageId, ProjectId, ScheduleId, UserId};

#[async_trait]
pub trait ChatRepository: Send + Sync {
    /// Most recently updated first (`updated_at DESC, id DESC`), from `query.cursor` on, at most
    /// `query.limit` chats, each with its unread and awaiting-approval flags.
    async fn list(&self, user: UserId, query: ChatQuery) -> DomainResult<Page<ChatSummary>>;
    async fn get(&self, user: UserId, id: ChatId) -> DomainResult<Option<Chat>>;
    /// With its `chat_tool_servers` rows, in one transaction.
    async fn insert(&self, chat: &Chat) -> DomainResult<()>;
    /// Writes the chat's settings and replaces its `chat_tool_servers` rows, in one
    /// transaction. `updated_at` and `last_read_at` are not written here: message writes move
    /// `updated_at`, and `mark_read` sets `last_read_at`.
    async fn update(&self, chat: &Chat) -> DomainResult<()>;
    /// Cascades to its messages, snapshots, attachments, and join rows; schedules created from
    /// it keep existing (`source_chat_id` becomes null). `false` when the user has no such chat.
    async fn delete(&self, user: UserId, id: ChatId) -> DomainResult<bool>;
    /// Sets `last_read_at = at`.
    async fn mark_read(&self, user: UserId, id: ChatId, at: DateTime<Utc>) -> DomainResult<()>;
}

/// Every write here also sets the chat's `updated_at` to the message's `updated_at` (or the
/// repository's clock for deletes), which drives the recent list and the unread dot.
#[async_trait]
pub trait MessageRepository: Send + Sync {
    /// The chat's messages, ordered by `seq`.
    async fn history(&self, user: UserId, chat: ChatId) -> DomainResult<Vec<Message>>;
    /// Inserts the message (next `seq`) and bumps the chat, in one transaction.
    async fn append(&self, user: UserId, message: &Message) -> DomainResult<()>;
    async fn update(&self, user: UserId, message: &Message) -> DomainResult<()>;
    /// Edit and resend, in one transaction: deletes `from` and every later message of the
    /// chat, then appends `message`.
    async fn replace_tail(
        &self,
        user: UserId,
        chat: ChatId,
        from: MessageId,
        message: &Message,
    ) -> DomainResult<()>;
    /// Deletes every message of the chat after `after`; returns how many.
    async fn delete_after(&self, user: UserId, chat: ChatId, after: MessageId)
    -> DomainResult<u64>;
    /// One guarded update, applied only while the message is `awaiting_approval`: each listed
    /// pending call becomes `approved` or `denied` and the message becomes `complete`
    /// (`Message::decided` is the reference). `false` when the message was not awaiting
    /// approval, so each decision applies once.
    async fn decide(
        &self,
        user: UserId,
        message: MessageId,
        decisions: &[CallDecision],
    ) -> DomainResult<bool>;
    /// Chats whose title or message text matches, best first.
    async fn search(&self, user: UserId, query: &str, limit: u32) -> DomainResult<Vec<SearchHit>>;
    /// System, at startup: every `streaming` message becomes `error` with the text
    /// `INTERRUPTED`. Returns how many.
    async fn recover(&self, now: DateTime<Utc>) -> DomainResult<u64>;
}

#[async_trait]
pub trait TurnContextRepository: Send + Sync {
    /// Inserts the snapshot unless the chat already has one with this hash.
    async fn save(&self, user: UserId, chat: ChatId, context: &TurnContext) -> DomainResult<()>;
    async fn get(
        &self,
        user: UserId,
        chat: ChatId,
        hash: &ContextHash,
    ) -> DomainResult<Option<TurnContext>>;
}

/// Which chats to list; the filters combine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChatQuery {
    pub project: Option<ProjectId>,
    /// The runs of one schedule.
    pub schedule: Option<ScheduleId>,
    pub pinned: Option<bool>,
    /// Only chats created by a schedule run (`Some(true)`, the Scheduled list) or only the
    /// others (`Some(false)`, Recent chats).
    pub scheduled: Option<bool>,
    /// Continue after this chat (the `next` of the previous page).
    pub cursor: Option<ChatCursor>,
    pub limit: u32,
}

impl Default for ChatQuery {
    fn default() -> Self {
        Self {
            project: None,
            schedule: None,
            pinned: None,
            scheduled: None,
            cursor: None,
            limit: DEFAULT_PAGE_SIZE,
        }
    }
}

/// A position in the chat list: the last chat of a page.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ChatCursor {
    pub updated_at: DateTime<Utc>,
    pub id: ChatId,
}

/// One page of a list; `next` is set when more items follow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<ChatCursor>,
}

/// A chat row of the sidebar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatSummary {
    pub chat: Chat,
    /// Changed since the user last opened it.
    pub unread: bool,
    /// Its latest assistant message waits for a tool decision.
    pub awaiting_approval: bool,
}

/// A search result: a chat title match (`message_id` is `None`) or a message match.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchHit {
    pub chat_id: ChatId,
    pub chat_title: String,
    pub message_id: Option<MessageId>,
    pub snippet: String,
}
