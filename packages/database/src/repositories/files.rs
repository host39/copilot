//! `FileRepository` on Postgres (`files`, `file_contents`). Roadmap step P2.A writes the queries.

use async_trait::async_trait;
use bytes::Bytes;
use copilot_core::{
    DomainResult, FileId, FileOwner, FileRepository, ProjectId, StoredFile, UserId,
};

use crate::repositories::store::{PgStore, pending};

#[async_trait]
impl FileRepository for PgStore {
    async fn list(&self, _user: UserId, _owner: FileOwner) -> DomainResult<Vec<StoredFile>> {
        pending("FileRepository::list")
    }

    async fn get(&self, _user: UserId, _id: FileId) -> DomainResult<Option<StoredFile>> {
        pending("FileRepository::get")
    }

    async fn create(&self, _file: &StoredFile, _bytes: Bytes) -> DomainResult<()> {
        pending("FileRepository::create")
    }

    async fn content(&self, _user: UserId, _id: FileId) -> DomainResult<Option<Bytes>> {
        pending("FileRepository::content")
    }

    async fn update(&self, _file: &StoredFile) -> DomainResult<()> {
        pending("FileRepository::update")
    }

    async fn move_to(
        &self,
        _user: UserId,
        _ids: &[FileId],
        _project: ProjectId,
    ) -> DomainResult<()> {
        pending("FileRepository::move_to")
    }

    async fn delete(&self, _user: UserId, _id: FileId) -> DomainResult<bool> {
        pending("FileRepository::delete")
    }

    async fn usage(&self, _user: UserId) -> DomainResult<u64> {
        pending("FileRepository::usage")
    }
}
