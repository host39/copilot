//! `FileRepository` on Postgres (`files`, `file_contents`). A file's row and its bytes are
//! written in one transaction; the bytes are read only by `content`, never with the metadata.

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use copilot_core::{
    ChatId, DomainError, DomainResult, FileId, FileOwner, FileRepository, ProjectId, StoredFile,
    StoredFileParts, UserId,
};
use uuid::Uuid;

use crate::repositories::store::{PgStore, failed, uint};

const FILE_COLUMNS: &str = "created_at, size_bytes, token_estimate, page_count, id, user_id, \
     project_id, chat_id, included, name, media_type, sha256";

#[derive(sqlx::FromRow)]
struct FileRow {
    created_at: DateTime<Utc>,
    size_bytes: i64,
    token_estimate: i32,
    page_count: Option<i32>,
    id: Uuid,
    user_id: Uuid,
    project_id: Option<Uuid>,
    chat_id: Option<Uuid>,
    included: bool,
    name: String,
    media_type: String,
    sha256: Vec<u8>,
}

/// A stored value the `CHECK` constraints should have refused.
fn corrupt(key: &'static str) -> DomainError {
    DomainError::Internal {
        key: key.to_owned(),
        source: None,
    }
}

impl TryFrom<FileRow> for StoredFile {
    type Error = DomainError;

    fn try_from(row: FileRow) -> DomainResult<Self> {
        let owner = match (row.project_id, row.chat_id) {
            (Some(project), None) => FileOwner::Project(ProjectId::new(project)),
            (None, Some(chat)) => FileOwner::Chat(ChatId::new(chat)),
            _ => return Err(corrupt("database.files.owner.invalid")),
        };
        let sha256 = <[u8; 32]>::try_from(row.sha256.as_slice())
            .map_err(|_| corrupt("database.files.sha256.invalid"))?;
        Ok(Self::restore(StoredFileParts {
            id: FileId::new(row.id),
            user_id: UserId::new(row.user_id),
            owner,
            name: row.name,
            media_type: row.media_type,
            size_bytes: u64::try_from(row.size_bytes)
                .map_err(|_| corrupt("database.files.size-bytes.invalid"))?,
            sha256,
            token_estimate: uint(Some(row.token_estimate))
                .ok_or_else(|| corrupt("database.files.token-estimate.invalid"))?,
            page_count: uint(row.page_count),
            included: row.included,
            created_at: row.created_at,
        }))
    }
}

/// The `project_id` and `chat_id` columns of an owner.
fn columns(owner: FileOwner) -> (Option<Uuid>, Option<Uuid>) {
    match owner {
        FileOwner::Project(project) => (Some(project.uuid()), None),
        FileOwner::Chat(chat) => (None, Some(chat.uuid())),
    }
}

#[async_trait]
impl FileRepository for PgStore {
    async fn list(&self, user: UserId, owner: FileOwner) -> DomainResult<Vec<StoredFile>> {
        let (column, id) = match owner {
            FileOwner::Project(project) => ("project_id", project.uuid()),
            FileOwner::Chat(chat) => ("chat_id", chat.uuid()),
        };
        let sql = format!(
            "SELECT {FILE_COLUMNS} FROM files WHERE {column} = $1 AND user_id = $2 \
             ORDER BY created_at, id"
        );
        let rows: Vec<FileRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(id)
            .bind(user.uuid())
            .fetch_all(self.pool())
            .await
            .map_err(failed("database.files.list.failed"))?;
        rows.into_iter().map(StoredFile::try_from).collect()
    }

    async fn get(&self, user: UserId, id: FileId) -> DomainResult<Option<StoredFile>> {
        let sql = format!("SELECT {FILE_COLUMNS} FROM files WHERE id = $1 AND user_id = $2");
        let row: Option<FileRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(id.uuid())
            .bind(user.uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(failed("database.files.get.failed"))?;
        row.map(StoredFile::try_from).transpose()
    }

    async fn create(&self, file: &StoredFile, bytes: Bytes, quota: u64) -> DomainResult<bool> {
        let key = "database.files.create.failed";
        let (project, chat) = columns(file.owner());
        let mut tx = self.pool().begin().await.map_err(failed(key))?;
        // One upload of the user at a time checks the quota and inserts, so uploads at once
        // cannot all fit the same room. The two-key form keeps clear of the single-key locks
        // that sqlx and lib-rustauth take.
        sqlx::query(
            "SELECT pg_advisory_xact_lock(hashtext('copilot.files.quota'), hashtext($1::text))",
        )
        .bind(file.user_id().uuid())
        .execute(&mut *tx)
        .await
        .map_err(failed(key))?;
        let used: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(size_bytes), 0)::bigint FROM files WHERE user_id = $1",
        )
        .bind(file.user_id().uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(failed(key))?;
        let used = u64::try_from(used).unwrap_or_default();
        if used.saturating_add(file.size_bytes()) > quota {
            return Ok(false);
        }
        sqlx::query(
            "INSERT INTO files (created_at, size_bytes, token_estimate, page_count, id, user_id, \
             project_id, chat_id, included, name, media_type, sha256) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(file.created_at())
        .bind(i64::try_from(file.size_bytes()).unwrap_or(i64::MAX))
        .bind(i32::try_from(file.token_estimate()).unwrap_or(i32::MAX))
        .bind(
            file.page_count()
                .map(|pages| i32::try_from(pages).unwrap_or(i32::MAX)),
        )
        .bind(file.id().uuid())
        .bind(file.user_id().uuid())
        .bind(project)
        .bind(chat)
        .bind(file.included())
        .bind(file.name())
        .bind(file.media_type())
        .bind(file.sha256().as_slice())
        .execute(&mut *tx)
        .await
        .map_err(failed(key))?;
        sqlx::query("INSERT INTO file_contents (file_id, bytes) VALUES ($1, $2)")
            .bind(file.id().uuid())
            .bind(bytes.as_ref())
            .execute(&mut *tx)
            .await
            .map_err(failed(key))?;
        tx.commit().await.map_err(failed(key))?;
        Ok(true)
    }

    async fn content(&self, user: UserId, id: FileId) -> DomainResult<Option<Bytes>> {
        let bytes: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT c.bytes FROM file_contents c JOIN files f ON f.id = c.file_id \
             WHERE c.file_id = $1 AND f.user_id = $2",
        )
        .bind(id.uuid())
        .bind(user.uuid())
        .fetch_optional(self.pool())
        .await
        .map_err(failed("database.files.content.failed"))?;
        Ok(bytes.map(Bytes::from))
    }

    async fn update(&self, file: &StoredFile) -> DomainResult<()> {
        sqlx::query("UPDATE files SET name = $3, included = $4 WHERE id = $1 AND user_id = $2")
            .bind(file.id().uuid())
            .bind(file.user_id().uuid())
            .bind(file.name())
            .bind(file.included())
            .execute(self.pool())
            .await
            .map_err(failed("database.files.update.failed"))?;
        Ok(())
    }

    async fn move_to(&self, user: UserId, ids: &[FileId], project: ProjectId) -> DomainResult<()> {
        let ids: Vec<Uuid> = ids.iter().map(|id| id.uuid()).collect();
        // The composite key `(project_id, user_id)` refuses a project of another user.
        sqlx::query(
            "UPDATE files SET project_id = $3, chat_id = NULL \
             WHERE id = ANY($1) AND user_id = $2",
        )
        .bind(&ids)
        .bind(user.uuid())
        .bind(project.uuid())
        .execute(self.pool())
        .await
        .map_err(failed("database.files.move.failed"))?;
        Ok(())
    }

    async fn delete(&self, user: UserId, id: FileId) -> DomainResult<bool> {
        let deleted = sqlx::query("DELETE FROM files WHERE id = $1 AND user_id = $2")
            .bind(id.uuid())
            .bind(user.uuid())
            .execute(self.pool())
            .await
            .map_err(failed("database.files.delete.failed"))?
            .rows_affected();
        Ok(deleted > 0)
    }

    async fn usage(&self, user: UserId) -> DomainResult<u64> {
        let used: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(size_bytes), 0)::bigint FROM files WHERE user_id = $1",
        )
        .bind(user.uuid())
        .fetch_one(self.pool())
        .await
        .map_err(failed("database.files.usage.failed"))?;
        Ok(u64::try_from(used).unwrap_or_default())
    }
}
