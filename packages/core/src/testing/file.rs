//! In-memory `FileRepository`.

use async_trait::async_trait;
use bytes::Bytes;

use crate::domains::file::{FileOwner, FileRepository, StoredFile};
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{FileId, ProjectId, UserId};
use crate::testing::store::{MemoryStore, Tables, duplicate};

#[async_trait]
impl FileRepository for MemoryStore {
    async fn list(&self, user: UserId, owner: FileOwner) -> DomainResult<Vec<StoredFile>> {
        let tables = self.tables();
        let mut files: Vec<StoredFile> = tables
            .files
            .values()
            .map(|(file, _)| file)
            .filter(|file| file.user_id() == user && file.owner() == owner)
            .cloned()
            .collect();
        files.sort_by_key(|file| (file.created_at(), file.id()));
        Ok(files)
    }

    async fn get(&self, user: UserId, id: FileId) -> DomainResult<Option<StoredFile>> {
        let tables = self.tables();
        Ok(tables
            .files
            .get(&id)
            .map(|(file, _)| file)
            .filter(|file| file.user_id() == user)
            .cloned())
    }

    async fn create(&self, file: &StoredFile, bytes: Bytes, quota: u64) -> DomainResult<bool> {
        let mut tables = self.tables();
        if tables.files.contains_key(&file.id()) {
            return Err(duplicate("files"));
        }
        tables.check_owner(file.user_id(), file.owner())?;
        let used: u64 = tables
            .files
            .values()
            .filter(|(stored, _)| stored.user_id() == file.user_id())
            .map(|(stored, _)| stored.size_bytes())
            .sum();
        if used.saturating_add(file.size_bytes()) > quota {
            return Ok(false);
        }
        tables.files.insert(file.id(), (file.clone(), bytes));
        Ok(true)
    }

    async fn content(&self, user: UserId, id: FileId) -> DomainResult<Option<Bytes>> {
        let tables = self.tables();
        Ok(tables
            .files
            .get(&id)
            .filter(|(file, _)| file.user_id() == user)
            .map(|(_, bytes)| bytes.clone()))
    }

    async fn update(&self, file: &StoredFile) -> DomainResult<()> {
        let mut tables = self.tables();
        if let Some((stored, _)) = tables.files.get_mut(&file.id())
            && stored.user_id() == file.user_id()
        {
            *stored = file.clone();
        }
        Ok(())
    }

    async fn move_to(&self, user: UserId, ids: &[FileId], project: ProjectId) -> DomainResult<()> {
        let mut tables = self.tables();
        tables.check_owner(user, FileOwner::Project(project))?;
        for id in ids {
            if let Some((stored, _)) = tables.files.get_mut(id)
                && stored.user_id() == user
            {
                *stored = stored.clone().moved(project);
            }
        }
        Ok(())
    }

    async fn delete(&self, user: UserId, id: FileId) -> DomainResult<bool> {
        let mut tables = self.tables();
        let owned = tables
            .files
            .get(&id)
            .is_some_and(|(file, _)| file.user_id() == user);
        if owned {
            tables.files.remove(&id);
        }
        Ok(owned)
    }

    async fn usage(&self, user: UserId) -> DomainResult<u64> {
        let tables = self.tables();
        Ok(tables
            .files
            .values()
            .filter(|(file, _)| file.user_id() == user)
            .map(|(file, _)| file.size_bytes())
            .sum())
    }
}

impl Tables {
    /// The composite foreign keys `(project_id, user_id)` and `(chat_id, user_id)`.
    fn check_owner(&self, user: UserId, owner: FileOwner) -> DomainResult<()> {
        let exists = match owner {
            FileOwner::Project(project) => self
                .projects
                .get(&project)
                .is_some_and(|stored| stored.user_id() == user),
            FileOwner::Chat(chat) => self
                .chats
                .get(&chat)
                .is_some_and(|stored| stored.user_id() == user),
        };
        if exists {
            Ok(())
        } else {
            Err(DomainError::Internal {
                key: "testing.files.foreign-key.owner".to_owned(),
                source: None,
            })
        }
    }
}
