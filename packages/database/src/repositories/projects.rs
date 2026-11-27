//! `ProjectRepository` on Postgres (`projects`, `project_tool_servers`). A project and its tool
//! server rows are written in one transaction; the composite foreign keys keep every row the
//! project owner's.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use copilot_core::{
    BuiltinTool, ChatId, DomainError, DomainResult, FileId, ModelId, Project, ProjectId,
    ProjectParts, ProjectQuery, ProjectRepository, ReasoningLevel, ToolServerId, UserId,
};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::repositories::store::{PgStore, failed, text};

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

/// The project's built-in tools as stored (`text[]`).
fn builtin_tools(project: &Project) -> Vec<&'static str> {
    project
        .builtin_tools()
        .iter()
        .map(|tool| tool.as_str())
        .collect()
}

/// Inserts `project`'s row and its tool server rows, inside `tx`.
async fn insert_row(
    tx: &mut Transaction<'_, Postgres>,
    project: &Project,
    key: &'static str,
) -> DomainResult<()> {
    sqlx::query(
        "INSERT INTO projects (created_at, updated_at, id, user_id, default_model_id, pinned, \
         name, icon, instructions, reasoning, builtin_tools) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(project.created_at())
    .bind(project.updated_at())
    .bind(project.id().uuid())
    .bind(project.user_id().uuid())
    .bind(project.default_model_id().map(ModelId::uuid))
    .bind(project.pinned())
    .bind(project.name())
    .bind(project.icon())
    .bind(project.instructions())
    .bind(project.reasoning().map(ReasoningLevel::as_str))
    .bind(builtin_tools(project))
    .execute(&mut **tx)
    .await
    .map_err(failed(key))?;
    insert_servers(tx, project, key).await
}

/// Adds `project`'s tool server rows, inside `tx`.
async fn insert_servers(
    tx: &mut Transaction<'_, Postgres>,
    project: &Project,
    key: &'static str,
) -> DomainResult<()> {
    let servers: Vec<Uuid> = project
        .tool_server_ids()
        .iter()
        .map(|id| id.uuid())
        .collect();
    if servers.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO project_tool_servers (project_id, tool_server_id, user_id) \
         SELECT $1, server, $2 FROM UNNEST($3::uuid[]) AS server",
    )
    .bind(project.id().uuid())
    .bind(project.user_id().uuid())
    .bind(&servers)
    .execute(&mut **tx)
    .await
    .map_err(failed(key))?;
    Ok(())
}

#[async_trait]
impl ProjectRepository for PgStore {
    async fn list(&self, user: UserId, query: ProjectQuery) -> DomainResult<Vec<Project>> {
        let sql = format!(
            "SELECT {PROJECT_COLUMNS} FROM projects p \
             WHERE p.user_id = $1 AND ($2::boolean IS NULL OR p.pinned = $2) \
             ORDER BY p.updated_at DESC, p.id DESC LIMIT $3"
        );
        let rows: Vec<ProjectRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(user.uuid())
            .bind(query.pinned)
            .bind(query.limit.map(i64::from))
            .fetch_all(self.pool())
            .await
            .map_err(failed("database.projects.list.failed"))?;
        rows.into_iter().map(Project::try_from).collect()
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

    async fn insert(&self, project: &Project) -> DomainResult<()> {
        let key = "database.projects.insert.failed";
        let mut tx = self.pool().begin().await.map_err(failed(key))?;
        insert_row(&mut tx, project, key).await?;
        tx.commit().await.map_err(failed(key))
    }

    async fn update(&self, project: &Project) -> DomainResult<()> {
        let key = "database.projects.update.failed";
        let mut tx = self.pool().begin().await.map_err(failed(key))?;
        let updated = sqlx::query(
            "UPDATE projects SET updated_at = $3, default_model_id = $4, pinned = $5, name = $6, \
             icon = $7, instructions = $8, reasoning = $9, builtin_tools = $10 \
             WHERE id = $1 AND user_id = $2",
        )
        .bind(project.id().uuid())
        .bind(project.user_id().uuid())
        .bind(project.updated_at())
        .bind(project.default_model_id().map(ModelId::uuid))
        .bind(project.pinned())
        .bind(project.name())
        .bind(project.icon())
        .bind(project.instructions())
        .bind(project.reasoning().map(ReasoningLevel::as_str))
        .bind(builtin_tools(project))
        .execute(&mut *tx)
        .await
        .map_err(failed(key))?
        .rows_affected();
        if updated == 0 {
            // Another user's project, or one deleted meanwhile: nothing to replace.
            return Ok(());
        }
        sqlx::query("DELETE FROM project_tool_servers WHERE project_id = $1 AND user_id = $2")
            .bind(project.id().uuid())
            .bind(project.user_id().uuid())
            .execute(&mut *tx)
            .await
            .map_err(failed(key))?;
        insert_servers(&mut tx, project, key).await?;
        tx.commit().await.map_err(failed(key))
    }

    async fn delete(&self, user: UserId, id: ProjectId) -> DomainResult<bool> {
        let deleted = sqlx::query("DELETE FROM projects WHERE id = $1 AND user_id = $2")
            .bind(id.uuid())
            .bind(user.uuid())
            .execute(self.pool())
            .await
            .map_err(failed("database.projects.delete.failed"))?
            .rows_affected();
        Ok(deleted > 0)
    }

    async fn insert_with_chat(
        &self,
        project: &Project,
        chat: ChatId,
        attachments: &[FileId],
    ) -> DomainResult<bool> {
        let key = "database.projects.save-chat.failed";
        let user = project.user_id().uuid();
        let mut tx = self.pool().begin().await.map_err(failed(key))?;
        insert_row(&mut tx, project, key).await?;
        // Chat settings changes never move `updated_at` (only message writes do). A concurrent
        // save of the same chat waits for this row lock, then finds `project_id` set and moves
        // nothing.
        let moved = sqlx::query(
            "UPDATE chats SET project_id = $3, instructions = '' \
             WHERE id = $1 AND user_id = $2 AND project_id IS NULL",
        )
        .bind(chat.uuid())
        .bind(user)
        .bind(project.id().uuid())
        .execute(&mut *tx)
        .await
        .map_err(failed(key))?
        .rows_affected();
        if moved == 0 {
            // Dropping `tx` rolls the project back.
            return Ok(false);
        }
        sqlx::query("DELETE FROM chat_tool_servers WHERE chat_id = $1 AND user_id = $2")
            .bind(chat.uuid())
            .bind(user)
            .execute(&mut *tx)
            .await
            .map_err(failed(key))?;
        let attachments: Vec<Uuid> = attachments.iter().map(|id| id.uuid()).collect();
        sqlx::query(
            "UPDATE files SET project_id = $3, chat_id = NULL \
             WHERE chat_id = $1 AND user_id = $2 AND id = ANY($4)",
        )
        .bind(chat.uuid())
        .bind(user)
        .bind(project.id().uuid())
        .bind(&attachments)
        .execute(&mut *tx)
        .await
        .map_err(failed(key))?;
        tx.commit().await.map_err(failed(key))?;
        Ok(true)
    }
}
