//! `ChatRepository`, `MessageRepository`, and `TurnContextRepository` on Postgres (`chats`,
//! `chat_tool_servers`, `chat_tool_approvals`, `messages`, `turn_contexts`). Roadmap step P2.A
//! writes the queries (P1.9 the ones the walking skeleton needs).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use copilot_core::{
    CallDecision, Chat, ChatId, ChatQuery, ChatRepository, ChatSummary, ContextHash, DomainResult,
    Message, MessageId, MessageRepository, Page, SearchHit, TurnContext, TurnContextRepository,
    UserId,
};

use crate::repositories::store::{PgStore, pending};

#[async_trait]
impl ChatRepository for PgStore {
    async fn list(&self, _user: UserId, _query: ChatQuery) -> DomainResult<Page<ChatSummary>> {
        pending("ChatRepository::list")
    }

    async fn get(&self, _user: UserId, _id: ChatId) -> DomainResult<Option<Chat>> {
        pending("ChatRepository::get")
    }

    async fn insert(&self, _chat: &Chat) -> DomainResult<()> {
        pending("ChatRepository::insert")
    }

    async fn update(&self, _chat: &Chat) -> DomainResult<()> {
        pending("ChatRepository::update")
    }

    async fn delete(&self, _user: UserId, _id: ChatId) -> DomainResult<bool> {
        pending("ChatRepository::delete")
    }

    async fn mark_read(&self, _user: UserId, _id: ChatId, _at: DateTime<Utc>) -> DomainResult<()> {
        pending("ChatRepository::mark_read")
    }
}

#[async_trait]
impl MessageRepository for PgStore {
    async fn history(&self, _user: UserId, _chat: ChatId) -> DomainResult<Vec<Message>> {
        pending("MessageRepository::history")
    }

    async fn append(&self, _user: UserId, _message: &Message) -> DomainResult<()> {
        pending("MessageRepository::append")
    }

    async fn update(&self, _user: UserId, _message: &Message) -> DomainResult<()> {
        pending("MessageRepository::update")
    }

    async fn replace_tail(
        &self,
        _user: UserId,
        _chat: ChatId,
        _from: MessageId,
        _message: &Message,
    ) -> DomainResult<()> {
        pending("MessageRepository::replace_tail")
    }

    async fn delete_after(
        &self,
        _user: UserId,
        _chat: ChatId,
        _after: MessageId,
    ) -> DomainResult<u64> {
        pending("MessageRepository::delete_after")
    }

    async fn decide(
        &self,
        _user: UserId,
        _message: MessageId,
        _decisions: &[CallDecision],
    ) -> DomainResult<bool> {
        pending("MessageRepository::decide")
    }

    async fn search(
        &self,
        _user: UserId,
        _query: &str,
        _limit: u32,
    ) -> DomainResult<Vec<SearchHit>> {
        pending("MessageRepository::search")
    }

    async fn recover(&self, _now: DateTime<Utc>) -> DomainResult<u64> {
        pending("MessageRepository::recover")
    }
}

#[async_trait]
impl TurnContextRepository for PgStore {
    async fn save(&self, _user: UserId, _chat: ChatId, _context: &TurnContext) -> DomainResult<()> {
        pending("TurnContextRepository::save")
    }

    async fn get(
        &self,
        _user: UserId,
        _chat: ChatId,
        _hash: &ContextHash,
    ) -> DomainResult<Option<TurnContext>> {
        pending("TurnContextRepository::get")
    }
}
