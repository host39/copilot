//! In-memory `ChatRepository`, `MessageRepository`, and `TurnContextRepository`.

use std::cmp::Reverse;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domains::chat::{
    CallDecision, Chat, ChatCursor, ChatParts, ChatQuery, ChatRepository, ChatSummary, ContextHash,
    INTERRUPTED, Message, MessageParts, MessageRepository, MessageStatus, Page, Role, SearchHit,
    TurnContext, TurnContextRepository,
};
use crate::domains::file::FileOwner;
use crate::domains::schedule::{Schedule, ScheduleParts};
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{ChatId, MessageId, ToolServerId, UserId};
use crate::testing::schedule::parts as schedule_parts;
use crate::testing::store::{MemoryStore, Tables, duplicate};

/// Characters of message text a search snippet shows.
const SNIPPET_CHARS: usize = 100;

#[async_trait]
impl ChatRepository for MemoryStore {
    async fn list(&self, user: UserId, query: ChatQuery) -> DomainResult<Page<ChatSummary>> {
        let tables = self.tables();
        let mut chats: Vec<&Chat> = tables
            .chats
            .values()
            .filter(|chat| chat.user_id() == user)
            .filter(|chat| query.project.is_none_or(|id| chat.project_id() == Some(id)))
            .filter(|chat| {
                query
                    .schedule
                    .is_none_or(|id| chat.schedule_id() == Some(id))
            })
            .filter(|chat| query.pinned.is_none_or(|pinned| chat.pinned() == pinned))
            .filter(|chat| {
                query
                    .scheduled
                    .is_none_or(|scheduled| chat.schedule_id().is_some() == scheduled)
            })
            .filter(|chat| {
                query.cursor.is_none_or(|cursor| {
                    (chat.updated_at(), chat.id()) < (cursor.updated_at, cursor.id)
                })
            })
            .collect();
        chats.sort_by_key(|chat| Reverse((chat.updated_at(), chat.id())));
        let more = chats.len() > query.limit as usize;
        chats.truncate(query.limit as usize);
        let next = more.then(|| {
            let last = chats.last().expect("a full page has a last chat");
            ChatCursor {
                updated_at: last.updated_at(),
                id: last.id(),
            }
        });
        let items = chats
            .into_iter()
            .map(|chat| ChatSummary {
                chat: chat.clone(),
                unread: chat.unread(),
                awaiting_approval: tables
                    .chat_messages(chat.id())
                    .rev()
                    .find(|message| message.role() == Role::Assistant)
                    .is_some_and(|message| message.status() == MessageStatus::AwaitingApproval),
            })
            .collect();
        Ok(Page { items, next })
    }

    async fn get(&self, user: UserId, id: ChatId) -> DomainResult<Option<Chat>> {
        let tables = self.tables();
        Ok(tables
            .chats
            .get(&id)
            .filter(|chat| chat.user_id() == user)
            .cloned())
    }

    async fn insert(&self, chat: &Chat) -> DomainResult<()> {
        let mut tables = self.tables();
        if tables.chats.contains_key(&chat.id()) {
            return Err(duplicate("chats"));
        }
        tables.check_chat_references(chat)?;
        tables.chats.insert(chat.id(), chat.clone());
        Ok(())
    }

    async fn update(&self, chat: &Chat) -> DomainResult<()> {
        let mut tables = self.tables();
        // Approvals only grow, and rows naming a server the user no longer has are skipped.
        let user = chat.user_id();
        let owned = |tables: &Tables, id: ToolServerId| {
            tables
                .tool_servers
                .get(&id)
                .is_some_and(|server| server.user_id() == user)
        };
        let mut parts = chat.to_parts();
        if let Some(stored) = tables.chats.get(&chat.id()) {
            for tool in stored.approved_tools() {
                if !parts.approved_tools.contains(tool) {
                    parts.approved_tools.push(tool.clone());
                }
            }
        }
        parts
            .approved_tools
            .retain(|tool| owned(&tables, tool.server_id));
        parts
            .tool_overrides
            .retain(|row| owned(&tables, row.server_id));
        let chat = Chat::restore(parts);
        tables.check_chat_references(&chat)?;
        if let Some(stored) = tables.chats.get_mut(&chat.id())
            && stored.user_id() == user
        {
            *stored = Chat::restore(ChatParts {
                updated_at: stored.updated_at(),
                last_read_at: stored.last_read_at(),
                ..chat.to_parts()
            });
        }
        Ok(())
    }

    async fn delete(&self, user: UserId, id: ChatId) -> DomainResult<bool> {
        let mut tables = self.tables();
        let owned = tables
            .chats
            .get(&id)
            .is_some_and(|chat| chat.user_id() == user);
        if owned {
            tables.delete_chat(id);
        }
        Ok(owned)
    }

    async fn mark_read(&self, user: UserId, id: ChatId, at: DateTime<Utc>) -> DomainResult<()> {
        let mut tables = self.tables();
        if let Some(chat) = tables.chats.get_mut(&id)
            && chat.user_id() == user
        {
            *chat = Chat::restore(ChatParts {
                last_read_at: Some(at),
                ..chat.to_parts()
            });
        }
        Ok(())
    }
}

#[async_trait]
impl MessageRepository for MemoryStore {
    async fn history(&self, user: UserId, chat: ChatId) -> DomainResult<Vec<Message>> {
        let tables = self.tables();
        Ok(tables
            .chat_messages(chat)
            .filter(|message| message.user_id() == user)
            .cloned()
            .collect())
    }

    async fn append(&self, user: UserId, message: &Message) -> DomainResult<()> {
        let mut tables = self.tables();
        tables.append_message(user, message)
    }

    async fn update(&self, user: UserId, message: &Message) -> DomainResult<()> {
        let mut tables = self.tables();
        let found = tables
            .messages
            .values_mut()
            .find(|stored| stored.id() == message.id() && stored.user_id() == user);
        if let Some(stored) = found {
            *stored = message.clone();
            tables.touch_chat(message.chat_id(), message.updated_at());
        }
        Ok(())
    }

    async fn replace_tail(
        &self,
        user: UserId,
        chat: ChatId,
        from: MessageId,
        message: &Message,
    ) -> DomainResult<()> {
        let mut tables = self.tables();
        if let Some(seq) = tables.seq(user, chat, from) {
            tables
                .messages
                .retain(|stored_seq, stored| stored.chat_id() != chat || *stored_seq < seq);
        }
        tables.append_message(user, message)
    }

    async fn delete_after(
        &self,
        user: UserId,
        chat: ChatId,
        after: MessageId,
    ) -> DomainResult<u64> {
        let now = self.now();
        let mut tables = self.tables();
        let Some(seq) = tables.seq(user, chat, after) else {
            return Ok(0);
        };
        let before = tables.messages.len();
        tables
            .messages
            .retain(|stored_seq, stored| stored.chat_id() != chat || *stored_seq <= seq);
        let deleted = (before - tables.messages.len()) as u64;
        if deleted > 0 {
            tables.touch_chat(chat, now);
        }
        Ok(deleted)
    }

    async fn decide(
        &self,
        user: UserId,
        message: MessageId,
        decisions: &[CallDecision],
    ) -> DomainResult<bool> {
        let now = self.now();
        let mut tables = self.tables();
        let found = tables.messages.values_mut().find(|stored| {
            stored.id() == message
                && stored.user_id() == user
                && stored.status() == MessageStatus::AwaitingApproval
        });
        let Some(stored) = found else {
            return Ok(false);
        };
        *stored = stored.clone().decided(decisions, now);
        let chat = stored.chat_id();
        tables.touch_chat(chat, now);
        Ok(true)
    }

    async fn search(&self, user: UserId, query: &str, limit: u32) -> DomainResult<Vec<SearchHit>> {
        let tables = self.tables();
        let needle = query.to_lowercase();
        let mut chats: Vec<&Chat> = tables
            .chats
            .values()
            .filter(|chat| chat.user_id() == user)
            .collect();
        chats.sort_by_key(|chat| Reverse((chat.updated_at(), chat.id())));
        let mut hits = Vec::new();
        for chat in chats {
            if chat.title().to_lowercase().contains(&needle) {
                hits.push(SearchHit {
                    chat_id: chat.id(),
                    chat_title: chat.title().to_owned(),
                    message_id: None,
                    snippet: chat.title().to_owned(),
                });
            }
            for message in tables.chat_messages(chat.id()) {
                let text = message.text_content();
                if text.to_lowercase().contains(&needle) {
                    hits.push(SearchHit {
                        chat_id: chat.id(),
                        chat_title: chat.title().to_owned(),
                        message_id: Some(message.id()),
                        snippet: text.chars().take(SNIPPET_CHARS).collect(),
                    });
                }
            }
        }
        hits.truncate(limit as usize);
        Ok(hits)
    }

    async fn recover(&self, now: DateTime<Utc>) -> DomainResult<u64> {
        let mut tables = self.tables();
        let mut count = 0;
        for message in tables.messages.values_mut() {
            if message.status() == MessageStatus::Streaming {
                *message = Message::restore(MessageParts {
                    status: MessageStatus::Error,
                    error: Some(INTERRUPTED.to_owned()),
                    updated_at: now,
                    ..message.to_parts()
                });
                count += 1;
            }
        }
        Ok(count)
    }
}

#[async_trait]
impl TurnContextRepository for MemoryStore {
    async fn save(&self, user: UserId, chat: ChatId, context: &TurnContext) -> DomainResult<()> {
        let mut tables = self.tables();
        if !tables
            .chats
            .get(&chat)
            .is_some_and(|stored| stored.user_id() == user)
        {
            return Err(foreign_key("turn-contexts"));
        }
        tables
            .turn_contexts
            .entry((chat, *context.hash.as_bytes()))
            .or_insert_with(|| (user, context.clone()));
        Ok(())
    }

    async fn get(
        &self,
        user: UserId,
        chat: ChatId,
        hash: &ContextHash,
    ) -> DomainResult<Option<TurnContext>> {
        let tables = self.tables();
        Ok(tables
            .turn_contexts
            .get(&(chat, *hash.as_bytes()))
            .filter(|(owner, _)| *owner == user)
            .map(|(_, context)| context.clone()))
    }
}

impl Tables {
    /// A chat's messages in `seq` order.
    pub(crate) fn chat_messages(&self, chat: ChatId) -> impl DoubleEndedIterator<Item = &Message> {
        self.messages
            .values()
            .filter(move |message| message.chat_id() == chat)
    }

    fn seq(&self, user: UserId, chat: ChatId, id: MessageId) -> Option<u64> {
        self.messages
            .iter()
            .find(|(_, message)| {
                message.id() == id && message.chat_id() == chat && message.user_id() == user
            })
            .map(|(seq, _)| *seq)
    }

    /// The next `seq`, the foreign key to the chat, and the chat's `updated_at`.
    fn append_message(&mut self, user: UserId, message: &Message) -> DomainResult<()> {
        let owned = self
            .chats
            .get(&message.chat_id())
            .is_some_and(|chat| chat.user_id() == user && message.user_id() == user);
        if !owned {
            return Err(foreign_key("messages"));
        }
        if self
            .messages
            .values()
            .any(|stored| stored.id() == message.id())
        {
            return Err(duplicate("messages"));
        }
        self.next_seq += 1;
        self.messages.insert(self.next_seq, message.clone());
        self.touch_chat(message.chat_id(), message.updated_at());
        Ok(())
    }

    /// A message write moves the chat's `updated_at`.
    fn touch_chat(&mut self, id: ChatId, at: DateTime<Utc>) {
        if let Some(chat) = self.chats.get_mut(&id) {
            *chat = Chat::restore(ChatParts {
                updated_at: at,
                ..chat.to_parts()
            });
        }
    }

    /// The composite foreign keys of a chat: project, model, schedule, and tool servers.
    fn check_chat_references(&self, chat: &Chat) -> DomainResult<()> {
        let user = chat.user_id();
        let project_ok = chat.project_id().is_none_or(|id| {
            self.projects
                .get(&id)
                .is_some_and(|row| row.user_id() == user)
        });
        let model_ok = chat.model_id().is_none_or(|id| {
            self.models
                .get(&id)
                .is_some_and(|row| row.user_id() == user)
        });
        let schedule_ok = chat.schedule_id().is_none_or(|id| {
            self.schedules
                .get(&id)
                .is_some_and(|row| row.user_id() == user)
        });
        let owned = |id| {
            self.tool_servers
                .get(id)
                .is_some_and(|server| server.user_id() == user)
        };
        let servers_ok = chat
            .tool_overrides()
            .iter()
            .all(|row| owned(&row.server_id))
            && chat
                .approved_tools()
                .iter()
                .all(|tool| owned(&tool.server_id));
        if project_ok && model_ok && schedule_ok && servers_ok {
            Ok(())
        } else {
            Err(foreign_key("chats"))
        }
    }

    /// Deletes a chat and what cascades with it: messages, snapshots, and attachments; its
    /// schedules lose `source_chat_id`.
    pub(crate) fn delete_chat(&mut self, id: ChatId) {
        self.chats.remove(&id);
        self.messages.retain(|_, message| message.chat_id() != id);
        self.turn_contexts.retain(|(chat, _), _| *chat != id);
        self.files
            .retain(|_, (file, _)| file.owner() != FileOwner::Chat(id));
        for schedule in self.schedules.values_mut() {
            if schedule.source_chat_id() == Some(id) {
                *schedule = Schedule::restore(ScheduleParts {
                    source_chat_id: None,
                    ..schedule_parts(schedule)
                });
            }
        }
    }
}

fn foreign_key(table: &str) -> DomainError {
    DomainError::Internal {
        key: format!("testing.{table}.foreign-key"),
        source: None,
    }
}
