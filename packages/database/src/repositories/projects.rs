//! `ProjectRepository` on Postgres (`projects`, `project_tool_servers`). Roadmap step P2.A writes
//! the queries.

use async_trait::async_trait;
use copilot_core::{
    ChatId, DomainResult, Project, ProjectId, ProjectQuery, ProjectRepository, UserId,
};

use crate::repositories::store::{PgStore, pending};

#[async_trait]
impl ProjectRepository for PgStore {
    async fn list(&self, _user: UserId, _query: ProjectQuery) -> DomainResult<Vec<Project>> {
        pending("ProjectRepository::list")
    }

    async fn get(&self, _user: UserId, _id: ProjectId) -> DomainResult<Option<Project>> {
        pending("ProjectRepository::get")
    }

    async fn insert(&self, _project: &Project) -> DomainResult<()> {
        pending("ProjectRepository::insert")
    }

    async fn update(&self, _project: &Project) -> DomainResult<()> {
        pending("ProjectRepository::update")
    }

    async fn delete(&self, _user: UserId, _id: ProjectId) -> DomainResult<bool> {
        pending("ProjectRepository::delete")
    }

    async fn insert_with_chat(&self, _project: &Project, _chat: ChatId) -> DomainResult<()> {
        pending("ProjectRepository::insert_with_chat")
    }
}
