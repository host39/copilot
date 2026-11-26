//! `ProjectRepository` on Postgres (`projects`, `project_tool_servers`). Roadmap step P2.A writes
//! the remaining queries; `get` is written already, because creating a chat in a project
//! (`POST /chats` with `projectId`) checks that the caller owns the project.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use copilot_core::{
    BuiltinTool, ChatId, DomainError, DomainResult, ModelId, Project, ProjectId, ProjectParts,
    ProjectQuery, ProjectRepository, ReasoningLevel, ToolServerId, UserId,
};
use uuid::Uuid;

use crate::repositories::store::{PgStore, failed, pending, text};

const PROJECT_COLUMNS: &str = "p.created_at, p.updated_at, p.id, p.user_id, \
     p.default_model_id, p.pinned, p.name, p.icon, p.instructions, p.reasoning, \
     p.builtin_tools, \
     ARRAY(SELECT s.tool_server_id FROM project_tool_servers s WHERE s.project_id = p.id \
       ORDER BY s.tool_server_id) AS tool_server_ids";

/// A project with the ids of the tool servers it enables.
#[derive(sqlx::FromRow)]
struct ProjectRow {
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    id: Uuid,
    user_id: Uuid,
    default_model_id: Option<Uuid>,
    pinned: bool,
    name: String,
    icon: String,
    instructions: String,
    reasoning: Option<String>,
    builtin_tools: Vec<String>,
    tool_server_ids: Vec<Uuid>,
}

impl TryFrom<ProjectRow> for Project {
    type Error = DomainError;

    fn try_from(row: ProjectRow) -> DomainResult<Self> {
        let builtin_tools = row
            .builtin_tools
            .iter()
            .map(|tool| text::<BuiltinTool>(tool, "database.projects.builtin-tools.invalid"))
            .collect::<DomainResult<Vec<_>>>()?;
        let reasoning = row
            .reasoning
            .as_deref()
            .map(|level| text::<ReasoningLevel>(level, "database.projects.reasoning.invalid"))
            .transpose()?;
        Ok(Self::restore(ProjectParts {
            id: ProjectId::new(row.id),
            user_id: UserId::new(row.user_id),
            name: row.name,
            icon: row.icon,
            instructions: row.instructions,
            default_model_id: row.default_model_id.map(ModelId::new),
            reasoning,
            builtin_tools,
            tool_server_ids: row
                .tool_server_ids
                .into_iter()
                .map(ToolServerId::new)
                .collect(),
            pinned: row.pinned,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }))
    }
}

#[async_trait]
impl ProjectRepository for PgStore {
    async fn list(&self, _user: UserId, _query: ProjectQuery) -> DomainResult<Vec<Project>> {
        pending("ProjectRepository::list")
    }

    async fn get(&self, user: UserId, id: ProjectId) -> DomainResult<Option<Project>> {
        let sql =
            format!("SELECT {PROJECT_COLUMNS} FROM projects p WHERE p.id = $1 AND p.user_id = $2");
        let row: Option<ProjectRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(id.uuid())
            .bind(user.uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(failed("database.projects.get.failed"))?;
        row.map(Project::try_from).transpose()
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
