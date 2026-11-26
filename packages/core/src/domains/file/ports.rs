//! File persistence port. File bytes live in Postgres (`file_contents`), so one `pg_dump` is the
//! whole backup and deletes are transactional.

use async_trait::async_trait;
use bytes::Bytes;

use crate::domains::file::entities::{FileOwner, StoredFile};
use crate::shared::errors::DomainResult;
use crate::shared::ids::{FileId, ProjectId, UserId};

#[async_trait]
pub trait FileRepository: Send + Sync {
    /// The owner's files, oldest first.
    async fn list(&self, user: UserId, owner: FileOwner) -> DomainResult<Vec<StoredFile>>;
    async fn get(&self, user: UserId, id: FileId) -> DomainResult<Option<StoredFile>>;
    /// The `files` row and its `file_contents` row, in one transaction.
    async fn create(&self, file: &StoredFile, bytes: Bytes) -> DomainResult<()>;
    async fn content(&self, user: UserId, id: FileId) -> DomainResult<Option<Bytes>>;
    /// Writes `name` and `included`.
    async fn update(&self, file: &StoredFile) -> DomainResult<()>;
    /// Moves the files into `project` (sets `project_id`, clears `chat_id`); `included` is kept.
    async fn move_to(&self, user: UserId, ids: &[FileId], project: ProjectId) -> DomainResult<()>;
    /// `false` when the user has no such file.
    async fn delete(&self, user: UserId, id: FileId) -> DomainResult<bool>;
    /// Bytes stored by the user (`SUM(size_bytes)`).
    async fn usage(&self, user: UserId) -> DomainResult<u64>;
}
