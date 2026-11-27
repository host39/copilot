//! `ChatRepository`, `MessageRepository`, and `TurnContextRepository` on Postgres (`chats`,
//! `chat_tool_servers`, `chat_tool_approvals`, `messages`, `turn_contexts`). The walking skeleton
//! (P1.9) wrote what a turn needs and roadmap step P2.A the chat list, search, and the tail
//! changes; P3.1 writes decisions and startup recovery.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use copilot_core::{
    ApprovedTool, BuiltinTool, CallDecision, Chat, ChatCursor, ChatId, ChatParts, ChatQuery,
    ChatRepository, ChatSummary, ContextHash, DomainError, DomainResult, Message, MessageId,
    MessageParts, MessageRepository, ModelId, Page, Part, ProjectId, ProviderId, ReasoningLevel,
    ScheduleId, SearchHit, ServerOverride, SnapshotFile, SnapshotTool, ToolServerId, TurnContext,
    TurnContextRepository, Usage, UserId,
};
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::records::{
    PartRecord, ProviderStateRecord, SnapshotFileRecord, SnapshotToolRecord, UsageRecord,
};
use crate::repositories::store::{PgStore, failed, invalid, json, pending, record, text};

const CHAT_COLUMNS: &str = "c.created_at, c.updated_at, c.last_read_at, c.id, c.user_id, \
     c.project_id, c.model_id, c.schedule_id, c.pinned, c.voice_replies, c.title, \
     c.instructions, c.reasoning, c.builtin_tools, \
     ARRAY(SELECT s.tool_server_id FROM chat_tool_servers s WHERE s.chat_id = c.id \
       ORDER BY s.tool_server_id) AS override_ids, \
     ARRAY(SELECT s.enabled FROM chat_tool_servers s WHERE s.chat_id = c.id \
       ORDER BY s.tool_server_id) AS override_enabled, \
     ARRAY(SELECT a.tool_server_id FROM chat_tool_approvals a WHERE a.chat_id = c.id \
       ORDER BY a.tool_server_id, a.tool_name) AS approval_ids, \
     ARRAY(SELECT a.tool_name FROM chat_tool_approvals a WHERE a.chat_id = c.id \
       ORDER BY a.tool_server_id, a.tool_name) AS approval_names";

/// A chat list row: the chat with its sidebar flags. `unread`: changed since the user last
/// opened it. `awaiting_approval`: its latest assistant message waits for a tool decision (the
/// partial index on open messages finds the candidates).
const SUMMARY_COLUMNS: &str = "(c.last_read_at IS NULL OR c.updated_at > c.last_read_at) \
     AS unread, \
     EXISTS (SELECT 1 FROM messages m WHERE m.chat_id = c.id AND m.user_id = c.user_id \
       AND m.status = 'awaiting_approval' AND m.role = 'assistant' \
       AND NOT EXISTS (SELECT 1 FROM messages later WHERE later.chat_id = m.chat_id \
         AND later.role = 'assistant' AND later.seq > m.seq)) AS awaiting_approval";

#[derive(sqlx::FromRow)]
struct SummaryRow {
    #[sqlx(flatten)]
    chat: ChatRow,
    unread: bool,
    awaiting_approval: bool,
}

/// A chat with its tool overrides and approvals, each as parallel arrays.
#[derive(sqlx::FromRow)]
struct ChatRow {
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    last_read_at: Option<DateTime<Utc>>,
    id: Uuid,
    user_id: Uuid,
    project_id: Option<Uuid>,
    model_id: Option<Uuid>,
    schedule_id: Option<Uuid>,
    pinned: bool,
    voice_replies: bool,
    title: String,
    instructions: String,
    reasoning: Option<String>,
    builtin_tools: Option<Vec<String>>,
    override_ids: Vec<Uuid>,
    override_enabled: Vec<bool>,
    approval_ids: Vec<Uuid>,
    approval_names: Vec<String>,
}

impl TryFrom<ChatRow> for Chat {
    type Error = DomainError;

    fn try_from(row: ChatRow) -> DomainResult<Self> {
        let builtin_tools = row
            .builtin_tools
            .map(|tools| {
                tools
                    .iter()
                    .map(|tool| text::<BuiltinTool>(tool, "database.chats.builtin-tools.invalid"))
                    .collect::<DomainResult<Vec<_>>>()
            })
            .transpose()?;
        Ok(Self::restore(ChatParts {
            id: ChatId::new(row.id),
            user_id: UserId::new(row.user_id),
            project_id: row.project_id.map(ProjectId::new),
            model_id: row.model_id.map(ModelId::new),
            schedule_id: row.schedule_id.map(ScheduleId::new),
            title: row.title,
            instructions: row.instructions,
            reasoning: reasoning(row.reasoning.as_deref(), "database.chats.reasoning.invalid")?,
            builtin_tools,
            approved_tools: row
                .approval_ids
                .into_iter()
                .zip(row.approval_names)
                .map(|(server, name)| ApprovedTool {
                    server_id: ToolServerId::new(server),
                    name,
                })
                .collect(),
            tool_overrides: row
                .override_ids
                .into_iter()
                .zip(row.override_enabled)
                .map(|(server, enabled)| ServerOverride {
                    server_id: ToolServerId::new(server),
                    enabled,
                })
                .collect(),
            pinned: row.pinned,
            voice_replies: row.voice_replies,
            last_read_at: row.last_read_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }))
    }
}

fn reasoning(value: Option<&str>, key: &'static str) -> DomainResult<Option<ReasoningLevel>> {
    value.map(|level| text(level, key)).transpose()
}

fn builtin(chat: &Chat) -> Option<Vec<&'static str>> {
    chat.builtin_tools()
        .map(|tools| tools.iter().map(|tool| tool.as_str()).collect())
}

/// Writes the chat's `chat_tool_servers` and `chat_tool_approvals` rows, skipping servers the
/// user no longer has (deleted since the chat was read; their rows went with them).
async fn join_rows(tx: &mut Transaction<'_, Postgres>, chat: &Chat) -> DomainResult<()> {
    let overrides = chat.tool_overrides();
    if !overrides.is_empty() {
        let servers: Vec<Uuid> = overrides.iter().map(|row| row.server_id.uuid()).collect();
        let enabled: Vec<bool> = overrides.iter().map(|row| row.enabled).collect();
        sqlx::query(
            "INSERT INTO chat_tool_servers (chat_id, tool_server_id, user_id, enabled) \
             SELECT $1, rows.server, $2, rows.enabled \
             FROM UNNEST($3::uuid[], $4::boolean[]) AS rows (server, enabled) \
             WHERE EXISTS (SELECT 1 FROM tool_servers t \
               WHERE t.id = rows.server AND t.user_id = $2)",
        )
        .bind(chat.id().uuid())
        .bind(chat.user_id().uuid())
        .bind(&servers)
        .bind(&enabled)
        .execute(&mut **tx)
        .await
        .map_err(failed("database.chats.tool-servers.failed"))?;
    }
    let approvals = chat.approved_tools();
    if !approvals.is_empty() {
        let servers: Vec<Uuid> = approvals.iter().map(|tool| tool.server_id.uuid()).collect();
        let names: Vec<&str> = approvals.iter().map(|tool| tool.name.as_str()).collect();
        sqlx::query(
            "INSERT INTO chat_tool_approvals (chat_id, tool_server_id, user_id, tool_name) \
             SELECT $1, rows.server, $2, rows.name \
             FROM UNNEST($3::uuid[], $4::text[]) AS rows (server, name) \
             WHERE EXISTS (SELECT 1 FROM tool_servers t \
               WHERE t.id = rows.server AND t.user_id = $2) \
             ON CONFLICT DO NOTHING",
        )
        .bind(chat.id().uuid())
        .bind(chat.user_id().uuid())
        .bind(&servers)
        .bind(&names)
        .execute(&mut **tx)
        .await
        .map_err(failed("database.chats.tool-approvals.failed"))?;
    }
    Ok(())
}

#[async_trait]
impl ChatRepository for PgStore {
    async fn list(&self, user: UserId, query: ChatQuery) -> DomainResult<Page<ChatSummary>> {
        let sql = format!(
            "SELECT {CHAT_COLUMNS}, {SUMMARY_COLUMNS} FROM chats c \
             WHERE c.user_id = $1 \
             AND ($2::uuid IS NULL OR c.project_id = $2) \
             AND ($3::uuid IS NULL OR c.schedule_id = $3) \
             AND ($4::boolean IS NULL OR c.pinned = $4) \
             AND ($5::boolean IS NULL OR (c.schedule_id IS NOT NULL) = $5) \
             AND ($6::timestamptz IS NULL OR (c.updated_at, c.id) < ($6, $7::uuid)) \
             ORDER BY c.updated_at DESC, c.id DESC LIMIT $8"
        );
        let limit = query.limit as usize;
        let rows: Vec<SummaryRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(user.uuid())
            .bind(query.project.map(ProjectId::uuid))
            .bind(query.schedule.map(ScheduleId::uuid))
            .bind(query.pinned)
            .bind(query.scheduled)
            .bind(query.cursor.map(|cursor| cursor.updated_at))
            .bind(query.cursor.map(|cursor| cursor.id.uuid()))
            // One more than the page, to know whether another page follows.
            .bind(i64::from(query.limit) + 1)
            .fetch_all(self.pool())
            .await
            .map_err(failed("database.chats.list.failed"))?;
        let more = rows.len() > limit;
        let items = rows
            .into_iter()
            .take(limit)
            .map(|row| {
                Ok(ChatSummary {
                    chat: Chat::try_from(row.chat)?,
                    unread: row.unread,
                    awaiting_approval: row.awaiting_approval,
                })
            })
            .collect::<DomainResult<Vec<_>>>()?;
        let next = if more {
            items.last().map(|last| ChatCursor {
                updated_at: last.chat.updated_at(),
                id: last.chat.id(),
            })
        } else {
            None
        };
        Ok(Page { items, next })
    }

    async fn get(&self, user: UserId, id: ChatId) -> DomainResult<Option<Chat>> {
        let sql = format!("SELECT {CHAT_COLUMNS} FROM chats c WHERE c.id = $1 AND c.user_id = $2");
        let row: Option<ChatRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(id.uuid())
            .bind(user.uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(failed("database.chats.get.failed"))?;
        row.map(Chat::try_from).transpose()
    }

    async fn insert(&self, chat: &Chat) -> DomainResult<()> {
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(failed("database.chats.insert.failed"))?;
        sqlx::query(
            "INSERT INTO chats (created_at, updated_at, last_read_at, id, user_id, project_id, \
             model_id, schedule_id, pinned, voice_replies, title, instructions, reasoning, \
             builtin_tools) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(chat.created_at())
        .bind(chat.updated_at())
        .bind(chat.last_read_at())
        .bind(chat.id().uuid())
        .bind(chat.user_id().uuid())
        .bind(chat.project_id().map(ProjectId::uuid))
        .bind(chat.model_id().map(ModelId::uuid))
        .bind(chat.schedule_id().map(ScheduleId::uuid))
        .bind(chat.pinned())
        .bind(chat.voice_replies())
        .bind(chat.title())
        .bind(chat.instructions())
        .bind(chat.reasoning().map(ReasoningLevel::as_str))
        .bind(builtin(chat))
        .execute(&mut *tx)
        .await
        .map_err(failed("database.chats.insert.failed"))?;
        join_rows(&mut tx, chat).await?;
        tx.commit()
            .await
            .map_err(failed("database.chats.insert.failed"))
    }

    async fn update(&self, chat: &Chat) -> DomainResult<()> {
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(failed("database.chats.update.failed"))?;
        let updated = sqlx::query(
            "UPDATE chats SET project_id = $3, model_id = $4, schedule_id = $5, pinned = $6, \
             voice_replies = $7, title = $8, instructions = $9, reasoning = $10, \
             builtin_tools = $11 WHERE id = $1 AND user_id = $2",
        )
        .bind(chat.id().uuid())
        .bind(chat.user_id().uuid())
        .bind(chat.project_id().map(ProjectId::uuid))
        .bind(chat.model_id().map(ModelId::uuid))
        .bind(chat.schedule_id().map(ScheduleId::uuid))
        .bind(chat.pinned())
        .bind(chat.voice_replies())
        .bind(chat.title())
        .bind(chat.instructions())
        .bind(chat.reasoning().map(ReasoningLevel::as_str))
        .bind(builtin(chat))
        .execute(&mut *tx)
        .await
        .map_err(failed("database.chats.update.failed"))?
        .rows_affected();
        if updated > 0 {
            // The overrides are the caller's to replace. Approvals only grow here: `chat` may
            // have been read before another request approved a tool for the chat.
            sqlx::query("DELETE FROM chat_tool_servers WHERE chat_id = $1 AND user_id = $2")
                .bind(chat.id().uuid())
                .bind(chat.user_id().uuid())
                .execute(&mut *tx)
                .await
                .map_err(failed("database.chats.update.failed"))?;
            join_rows(&mut tx, chat).await?;
        }
        tx.commit()
            .await
            .map_err(failed("database.chats.update.failed"))
    }

    async fn delete(&self, user: UserId, id: ChatId) -> DomainResult<bool> {
        let deleted = sqlx::query("DELETE FROM chats WHERE id = $1 AND user_id = $2")
            .bind(id.uuid())
            .bind(user.uuid())
            .execute(self.pool())
            .await
            .map_err(failed("database.chats.delete.failed"))?
            .rows_affected();
        Ok(deleted > 0)
    }

    async fn mark_read(&self, user: UserId, id: ChatId, at: DateTime<Utc>) -> DomainResult<()> {
        sqlx::query("UPDATE chats SET last_read_at = $3 WHERE id = $1 AND user_id = $2")
            .bind(id.uuid())
            .bind(user.uuid())
            .bind(at)
            .execute(self.pool())
            .await
            .map_err(failed("database.chats.mark-read.failed"))?;
        Ok(())
    }
}

const MESSAGE_COLUMNS: &str = "created_at, updated_at, id, user_id, chat_id, model_id, \
     provider_id, role, status, finish_reason, context_hash, parts, provider_state, usage, error";

#[derive(sqlx::FromRow)]
struct MessageRow {
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    id: Uuid,
    user_id: Uuid,
    chat_id: Uuid,
    model_id: Option<Uuid>,
    provider_id: Option<Uuid>,
    role: String,
    status: String,
    finish_reason: Option<String>,
    context_hash: Option<Vec<u8>>,
    parts: Value,
    provider_state: Option<Value>,
    usage: Option<Value>,
    error: Option<String>,
}

impl TryFrom<MessageRow> for Message {
    type Error = DomainError;

    fn try_from(row: MessageRow) -> DomainResult<Self> {
        let parts: Vec<PartRecord> = record(row.parts, "database.messages.parts.invalid")?;
        let parts = parts
            .into_iter()
            .map(Part::try_from)
            .collect::<Result<Vec<_>, _>>()
            .map_err(invalid("database.messages.parts.invalid"))?;
        let provider_state = row
            .provider_state
            .map(|value| {
                record::<ProviderStateRecord>(value, "database.messages.provider-state.invalid")?
                    .content()
                    .map_err(invalid("database.messages.provider-state.invalid"))
            })
            .transpose()?;
        let usage = row
            .usage
            .map(|value| {
                Usage::try_from(record::<UsageRecord>(
                    value,
                    "database.messages.usage.invalid",
                )?)
                .map_err(invalid("database.messages.usage.invalid"))
            })
            .transpose()?;
        let context_hash = row
            .context_hash
            .map(|bytes| {
                ContextHash::restore(&bytes).ok_or_else(|| DomainError::Internal {
                    key: "database.messages.context-hash.invalid".to_owned(),
                    source: None,
                })
            })
            .transpose()?;
        Ok(Self::restore(MessageParts {
            id: MessageId::new(row.id),
            user_id: UserId::new(row.user_id),
            chat_id: ChatId::new(row.chat_id),
            role: text(&row.role, "database.messages.role.invalid")?,
            status: text(&row.status, "database.messages.status.invalid")?,
            finish_reason: row
                .finish_reason
                .map(|reason| text(&reason, "database.messages.finish-reason.invalid"))
                .transpose()?,
            model_id: row.model_id.map(ModelId::new),
            provider_id: row.provider_id.map(ProviderId::new),
            context_hash,
            parts,
            provider_state,
            usage,
            error: row.error,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }))
    }
}

/// Full-text search over the user's chat titles and message text (`text_content`, whose GIN
/// index is on `to_tsvector('simple', text_content)`). Every word of the search text matches as a
/// prefix: Postgres parses the text into lexemes (`plainto_tsquery`, which accepts any input),
/// and each quoted lexeme gets `:*`. Title matches come first (most recently updated chat
/// first), then message matches by rank, newest first. Snippets are plain text: the title, or
/// the words around the match on one line.
const SEARCH: &str = r#"WITH q AS (
  SELECT to_tsquery('simple', regexp_replace(plainto_tsquery('simple', $2)::text,
    '''( |$)', ''':*\1', 'g')) AS query
),
hits AS (
  (SELECT c.id AS chat_id, NULL::uuid AS message_id, c.title AS body, 0 AS kind,
     1::real AS rank, c.updated_at AS at
   FROM chats c, q
   WHERE c.user_id = $1 AND to_tsvector('simple', c.title) @@ q.query
   ORDER BY c.updated_at DESC, c.id DESC LIMIT $3)
  UNION ALL
  (SELECT m.chat_id, m.id, m.text_content, 1,
     ts_rank(to_tsvector('simple', m.text_content), q.query), m.created_at
   FROM messages m, q
   WHERE m.user_id = $1 AND to_tsvector('simple', m.text_content) @@ q.query
   ORDER BY 5 DESC, m.created_at DESC, m.id DESC LIMIT $3)
),
best AS (
  SELECT * FROM hits
  ORDER BY kind, rank DESC, at DESC, chat_id DESC, message_id DESC
  LIMIT $3
)
SELECT b.chat_id, c.title AS chat_title, b.message_id,
  CASE WHEN b.message_id IS NULL THEN b.body
    ELSE btrim(regexp_replace(ts_headline('simple', b.body, q.query,
      'StartSel="", StopSel="", MaxWords=24, MinWords=8'), '\s+', ' ', 'g'))
  END AS snippet
FROM best b JOIN chats c ON c.id = b.chat_id AND c.user_id = $1, q
ORDER BY b.kind, b.rank DESC, b.at DESC, b.chat_id DESC, b.message_id DESC"#;

#[derive(sqlx::FromRow)]
struct SearchRow {
    chat_id: Uuid,
    chat_title: String,
    message_id: Option<Uuid>,
    snippet: String,
}

/// The stored `jsonb` values of a message: parts, provider state, and usage.
struct Stored {
    parts: Vec<PartRecord>,
    provider_state: Option<ProviderStateRecord>,
    usage: Option<UsageRecord>,
}

impl Stored {
    fn new(message: &Message) -> Self {
        Self {
            parts: message.parts().iter().map(PartRecord::from).collect(),
            provider_state: message.provider_state().map(|content| {
                ProviderStateRecord::new(content.clone(), message.provider_id(), message.model_id())
            }),
            usage: message.usage().copied().map(UsageRecord::from),
        }
    }
}

/// A message written for `user` must be the user's.
fn owned(user: UserId, message: &Message) -> DomainResult<()> {
    if message.user_id() == user {
        Ok(())
    } else {
        Err(DomainError::Internal {
            key: "database.messages.owner.mismatch".to_owned(),
            source: None,
        })
    }
}

/// Moves the chat to the top of the recent list (`updated_at`).
async fn touch(
    tx: &mut Transaction<'_, Postgres>,
    user: UserId,
    chat: ChatId,
    at: DateTime<Utc>,
) -> DomainResult<()> {
    sqlx::query("UPDATE chats SET updated_at = $3 WHERE id = $1 AND user_id = $2")
        .bind(chat.uuid())
        .bind(user.uuid())
        .bind(at)
        .execute(&mut **tx)
        .await
        .map_err(failed("database.chats.touch.failed"))?;
    Ok(())
}

/// Inserts `message` (the next `seq`), inside `tx`.
async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    user: UserId,
    message: &Message,
    key: &'static str,
) -> DomainResult<()> {
    let stored = Stored::new(message);
    sqlx::query(
        "INSERT INTO messages (created_at, updated_at, id, user_id, chat_id, model_id, \
         provider_id, role, status, finish_reason, context_hash, parts, provider_state, \
         usage, error, text_content) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
    )
    .bind(message.created_at())
    .bind(message.updated_at())
    .bind(message.id().uuid())
    .bind(user.uuid())
    .bind(message.chat_id().uuid())
    .bind(message.model_id().map(ModelId::uuid))
    .bind(message.provider_id().map(ProviderId::uuid))
    .bind(message.role().as_str())
    .bind(message.status().as_str())
    .bind(message.finish_reason().map(|reason| reason.as_str()))
    .bind(message.context_hash().map(|hash| hash.as_bytes().to_vec()))
    .bind(json(stored.parts))
    .bind(stored.provider_state.map(json))
    .bind(stored.usage.map(json))
    .bind(message.error())
    .bind(message.text_content())
    .execute(&mut **tx)
    .await
    .map_err(failed(key))?;
    Ok(())
}

#[async_trait]
impl MessageRepository for PgStore {
    async fn history(&self, user: UserId, chat: ChatId) -> DomainResult<Vec<Message>> {
        let sql = format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages WHERE chat_id = $1 AND user_id = $2 \
             ORDER BY seq"
        );
        let rows: Vec<MessageRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(chat.uuid())
            .bind(user.uuid())
            .fetch_all(self.pool())
            .await
            .map_err(failed("database.messages.history.failed"))?;
        rows.into_iter().map(Message::try_from).collect()
    }

    async fn append(&self, user: UserId, message: &Message) -> DomainResult<()> {
        owned(user, message)?;
        let key = "database.messages.append.failed";
        let mut tx = self.pool().begin().await.map_err(failed(key))?;
        insert(&mut tx, user, message, key).await?;
        touch(&mut tx, user, message.chat_id(), message.updated_at()).await?;
        tx.commit().await.map_err(failed(key))
    }

    async fn update(&self, user: UserId, message: &Message) -> DomainResult<()> {
        owned(user, message)?;
        let stored = Stored::new(message);
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(failed("database.messages.update.failed"))?;
        let updated = sqlx::query(
            "UPDATE messages SET updated_at = $3, model_id = $4, provider_id = $5, status = $6, \
             finish_reason = $7, context_hash = $8, parts = $9, provider_state = $10, \
             usage = $11, error = $12, text_content = $13 WHERE id = $1 AND user_id = $2",
        )
        .bind(message.id().uuid())
        .bind(user.uuid())
        .bind(message.updated_at())
        .bind(message.model_id().map(ModelId::uuid))
        .bind(message.provider_id().map(ProviderId::uuid))
        .bind(message.status().as_str())
        .bind(message.finish_reason().map(|reason| reason.as_str()))
        .bind(message.context_hash().map(|hash| hash.as_bytes().to_vec()))
        .bind(json(stored.parts))
        .bind(stored.provider_state.map(json))
        .bind(stored.usage.map(json))
        .bind(message.error())
        .bind(message.text_content())
        .execute(&mut *tx)
        .await
        .map_err(failed("database.messages.update.failed"))?
        .rows_affected();
        if updated > 0 {
            touch(&mut tx, user, message.chat_id(), message.updated_at()).await?;
        }
        tx.commit()
            .await
            .map_err(failed("database.messages.update.failed"))
    }

    async fn replace_tail(
        &self,
        user: UserId,
        chat: ChatId,
        from: MessageId,
        message: &Message,
    ) -> DomainResult<()> {
        owned(user, message)?;
        let key = "database.messages.replace-tail.failed";
        let mut tx = self.pool().begin().await.map_err(failed(key))?;
        // An unknown `from` matches nothing (`seq >= NULL`), so only the append happens.
        sqlx::query(
            "DELETE FROM messages WHERE chat_id = $1 AND user_id = $2 AND seq >= \
             (SELECT seq FROM messages WHERE id = $3 AND chat_id = $1 AND user_id = $2)",
        )
        .bind(chat.uuid())
        .bind(user.uuid())
        .bind(from.uuid())
        .execute(&mut *tx)
        .await
        .map_err(failed(key))?;
        insert(&mut tx, user, message, key).await?;
        touch(&mut tx, user, message.chat_id(), message.updated_at()).await?;
        tx.commit().await.map_err(failed(key))
    }

    async fn delete_after(
        &self,
        user: UserId,
        chat: ChatId,
        after: MessageId,
    ) -> DomainResult<u64> {
        let key = "database.messages.delete-after.failed";
        let mut tx = self.pool().begin().await.map_err(failed(key))?;
        let deleted = sqlx::query(
            "DELETE FROM messages WHERE chat_id = $1 AND user_id = $2 AND seq > \
             (SELECT seq FROM messages WHERE id = $3 AND chat_id = $1 AND user_id = $2)",
        )
        .bind(chat.uuid())
        .bind(user.uuid())
        .bind(after.uuid())
        .execute(&mut *tx)
        .await
        .map_err(failed(key))?
        .rows_affected();
        if deleted > 0 {
            touch(&mut tx, user, chat, self.now()).await?;
        }
        tx.commit().await.map_err(failed(key))?;
        Ok(deleted)
    }

    async fn decide(
        &self,
        _user: UserId,
        _message: MessageId,
        _decisions: &[CallDecision],
    ) -> DomainResult<bool> {
        pending("MessageRepository::decide")
    }

    async fn search(&self, user: UserId, query: &str, limit: u32) -> DomainResult<Vec<SearchHit>> {
        // Text without a letter or digit holds no word to look for (and Postgres would only
        // log a notice).
        if !query.chars().any(char::is_alphanumeric) {
            return Ok(Vec::new());
        }
        let rows: Vec<SearchRow> = sqlx::query_as(SEARCH)
            .bind(user.uuid())
            .bind(query)
            .bind(i64::from(limit))
            .fetch_all(self.pool())
            .await
            .map_err(failed("database.messages.search.failed"))?;
        Ok(rows
            .into_iter()
            .map(|row| SearchHit {
                chat_id: ChatId::new(row.chat_id),
                chat_title: row.chat_title,
                message_id: row.message_id.map(MessageId::new),
                snippet: row.snippet,
            })
            .collect())
    }

    async fn recover(&self, _now: DateTime<Utc>) -> DomainResult<u64> {
        pending("MessageRepository::recover")
    }
}

#[derive(sqlx::FromRow)]
struct ContextRow {
    window_start_id: Option<Uuid>,
    reasoning: Option<String>,
    hash: Vec<u8>,
    system: String,
    tools: Value,
    files: Value,
}

impl TryFrom<ContextRow> for TurnContext {
    type Error = DomainError;

    fn try_from(row: ContextRow) -> DomainResult<Self> {
        let tools: Vec<SnapshotToolRecord> =
            record(row.tools, "database.turn-contexts.tools.invalid")?;
        let files: Vec<SnapshotFileRecord> =
            record(row.files, "database.turn-contexts.files.invalid")?;
        Ok(Self {
            hash: ContextHash::restore(&row.hash).ok_or_else(|| DomainError::Internal {
                key: "database.turn-contexts.hash.invalid".to_owned(),
                source: None,
            })?,
            system: row.system,
            tools: tools
                .into_iter()
                .map(SnapshotTool::try_from)
                .collect::<Result<_, _>>()
                .map_err(invalid("database.turn-contexts.tools.invalid"))?,
            files: files
                .into_iter()
                .map(SnapshotFile::try_from)
                .collect::<Result<_, _>>()
                .map_err(invalid("database.turn-contexts.files.invalid"))?,
            window_start_id: row.window_start_id.map(MessageId::new),
            reasoning: reasoning(
                row.reasoning.as_deref(),
                "database.turn-contexts.reasoning.invalid",
            )?,
        })
    }
}

#[async_trait]
impl TurnContextRepository for PgStore {
    async fn save(&self, user: UserId, chat: ChatId, context: &TurnContext) -> DomainResult<()> {
        let tools: Vec<SnapshotToolRecord> =
            context.tools.iter().map(SnapshotToolRecord::from).collect();
        let files: Vec<SnapshotFileRecord> =
            context.files.iter().map(SnapshotFileRecord::from).collect();
        sqlx::query(
            "INSERT INTO turn_contexts (created_at, chat_id, user_id, window_start_id, \
             reasoning, hash, system, tools, files) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (chat_id, hash) DO NOTHING",
        )
        .bind(self.now())
        .bind(chat.uuid())
        .bind(user.uuid())
        .bind(context.window_start_id.map(MessageId::uuid))
        .bind(context.reasoning.map(ReasoningLevel::as_str))
        .bind(context.hash.as_bytes().to_vec())
        .bind(&context.system)
        .bind(json(tools))
        .bind(json(files))
        .execute(self.pool())
        .await
        .map_err(failed("database.turn-contexts.save.failed"))?;
        Ok(())
    }

    async fn get(
        &self,
        user: UserId,
        chat: ChatId,
        hash: &ContextHash,
    ) -> DomainResult<Option<TurnContext>> {
        let row: Option<ContextRow> = sqlx::query_as(
            "SELECT window_start_id, reasoning, hash, system, tools, files FROM turn_contexts \
             WHERE chat_id = $1 AND user_id = $2 AND hash = $3",
        )
        .bind(chat.uuid())
        .bind(user.uuid())
        .bind(hash.as_bytes().to_vec())
        .fetch_optional(self.pool())
        .await
        .map_err(failed("database.turn-contexts.get.failed"))?;
        row.map(TurnContext::try_from).transpose()
    }
}
