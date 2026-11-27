//! Project persistence port.

use async_trait::async_trait;

use crate::domains::project::entities::Project;
use crate::shared::errors::DomainResult;
use crate::shared::ids::{ChatId, FileId, ProjectId, UserId};

#[async_trait]
pub trait ProjectRepository: Send + Sync {
    /// Most recently updated first.
    async fn list(&self, user: UserId, query: ProjectQuery) -> DomainResult<Vec<Project>>;
    async fn get(&self, user: UserId, id: ProjectId) -> DomainResult<Option<Project>>;
    /// With its `project_tool_servers` rows, in one transaction.
    async fn insert(&self, project: &Project) -> DomainResult<()>;
    /// Replaces the `project_tool_servers` rows, in one transaction.
    async fn update(&self, project: &Project) -> DomainResult<()>;
    /// Cascades to the project's chats, files, and schedules. `false` when the user has no such
    /// project.
    async fn delete(&self, user: UserId, id: ProjectId) -> DomainResult<bool>;
    /// Save as project, in one transaction: inserts `project` with its tool server rows, moves the
    /// chat into it, moves those of `attachments` the chat owns into the project as included
    /// project files (no copy), and clears the chat's instructions and tool server overrides. The move applies only
    /// while the chat is still a standalone chat of the project's owner: otherwise (another
    /// request saved or moved it since the caller read it, or it is gone) nothing is written and
    /// the result is `false`.
    async fn insert_with_chat(
        &self,
        project: &Project,
        chat: ChatId,
        attachments: &[FileId],
    ) -> DomainResult<bool>;
}

/// Which projects to list.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProjectQuery {
    /// Only pinned (`Some(true)`) or only unpinned (`Some(false)`) projects.
    pub pinned: Option<bool>,
    /// At most this many (the sidebar shows the 5 most recent).
    pub limit: Option<u32>,
}
