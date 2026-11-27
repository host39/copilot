//! In-memory `ProjectRepository`.

use std::cmp::Reverse;

use async_trait::async_trait;

use crate::domains::file::FileOwner;
use crate::domains::project::{Project, ProjectQuery, ProjectRepository};
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{ChatId, ProjectId, UserId};
use crate::testing::store::{MemoryStore, Tables, duplicate};

#[async_trait]
impl ProjectRepository for MemoryStore {
    async fn list(&self, user: UserId, query: ProjectQuery) -> DomainResult<Vec<Project>> {
        let tables = self.tables();
        let mut projects: Vec<Project> = tables
            .projects
            .values()
            .filter(|project| project.user_id() == user)
            .filter(|project| query.pinned.is_none_or(|pinned| project.pinned() == pinned))
            .cloned()
            .collect();
        projects.sort_by_key(|project| Reverse((project.updated_at(), project.id())));
        if let Some(limit) = query.limit {
            projects.truncate(limit as usize);
        }
        Ok(projects)
    }

    async fn get(&self, user: UserId, id: ProjectId) -> DomainResult<Option<Project>> {
        let tables = self.tables();
        Ok(tables
            .projects
            .get(&id)
            .filter(|project| project.user_id() == user)
            .cloned())
    }

    async fn insert(&self, project: &Project) -> DomainResult<()> {
        let mut tables = self.tables();
        if tables.projects.contains_key(&project.id()) {
            return Err(duplicate("projects"));
        }
        tables.check_references(project)?;
        tables.projects.insert(project.id(), project.clone());
        Ok(())
    }

    async fn update(&self, project: &Project) -> DomainResult<()> {
        let mut tables = self.tables();
        tables.check_references(project)?;
        if let Some(stored) = tables.projects.get_mut(&project.id())
            && stored.user_id() == project.user_id()
        {
            *stored = project.clone();
        }
        Ok(())
    }

    async fn delete(&self, user: UserId, id: ProjectId) -> DomainResult<bool> {
        let mut tables = self.tables();
        if !tables
            .projects
            .get(&id)
            .is_some_and(|project| project.user_id() == user)
        {
            return Ok(false);
        }
        tables.delete_project(id);
        Ok(true)
    }

    async fn insert_with_chat(&self, project: &Project, chat: ChatId) -> DomainResult<bool> {
        let mut tables = self.tables();
        if tables.projects.contains_key(&project.id()) {
            return Err(duplicate("projects"));
        }
        // `UPDATE chats ... WHERE id = $1 AND user_id = $2 AND project_id IS NULL`.
        let Some(stored) = tables
            .chats
            .get(&chat)
            .filter(|stored| stored.user_id() == project.user_id())
            .filter(|stored| stored.project_id().is_none())
            .cloned()
        else {
            return Ok(false);
        };
        tables.check_references(project)?;
        tables.projects.insert(project.id(), project.clone());
        let moved = stored
            .with_project(Some(project.id()))
            .with_instructions("")
            .with_tool_overrides(Vec::new());
        tables.chats.insert(chat, moved);
        for (file, _) in tables.files.values_mut() {
            if file.owner() == FileOwner::Chat(chat) {
                *file = file.clone().moved(project.id());
            }
        }
        Ok(true)
    }
}

impl Tables {
    /// The composite foreign keys `(default_model_id, user_id) REFERENCES models (id, user_id)`
    /// and `(tool_server_id, user_id) REFERENCES tool_servers (id, user_id)`.
    fn check_references(&self, project: &Project) -> DomainResult<()> {
        let user = project.user_id();
        let model_ok = project.default_model_id().is_none_or(|model| {
            self.models
                .get(&model)
                .is_some_and(|stored| stored.user_id() == user)
        });
        if !model_ok {
            return Err(foreign_key("default-model"));
        }
        let servers_ok = project.tool_server_ids().iter().all(|server| {
            self.tool_servers
                .get(server)
                .is_some_and(|stored| stored.user_id() == user)
        });
        if !servers_ok {
            return Err(foreign_key("tool-server"));
        }
        Ok(())
    }
}

fn foreign_key(reference: &str) -> DomainError {
    DomainError::Internal {
        key: format!("testing.projects.foreign-key.{reference}"),
        source: None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ports::clock::Clock;
    use crate::shared::ids::ModelId;
    use crate::testing::{FixedClock, actor};

    #[tokio::test]
    async fn rejects_a_default_model_the_owner_does_not_have_like_the_foreign_key() {
        let clock = Arc::new(FixedClock::default());
        let store = MemoryStore::new(clock.clone());
        let project = Project::create(actor().user_id, "Work", clock.now())
            .unwrap()
            .with_default_model(Some(ModelId::generate()));
        let error = ProjectRepository::insert(&store, &project)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "INTERNAL_ERROR");
        let stored = ProjectRepository::get(&store, project.user_id(), project.id())
            .await
            .unwrap();
        assert!(stored.is_none());
    }
}
