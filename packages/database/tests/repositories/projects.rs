use copilot_core::{BuiltinTool, ProjectId, ProjectRepository, ReasoningLevel, ToolServerId};
use copilot_database::DbPool;
use uuid::Uuid;

use crate::{at, model, store, user};

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn a_project_is_read_with_its_tool_servers_by_its_owner_only(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let default_model = model(&store, owner).await;
    let server = Uuid::now_v7();
    let project = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO tool_servers (created_at, updated_at, id, user_id, name, slug, url, \
         headers_sealed) VALUES ($1, $1, $2, $3, 'Docs', 'docs', 'https://mcp.example.com', NULL)",
    )
    .bind(at(0))
    .bind(server)
    .bind(owner.uuid())
    .execute(store.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO projects (created_at, updated_at, id, user_id, default_model_id, pinned, \
         name, icon, instructions, reasoning, builtin_tools) \
         VALUES ($1, $2, $3, $4, $5, true, 'Research', 'R', 'Cite sources.', 'high', \
         ARRAY['schedules'])",
    )
    .bind(at(0))
    .bind(at(5))
    .bind(project)
    .bind(owner.uuid())
    .bind(default_model.id().uuid())
    .execute(store.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO project_tool_servers (project_id, tool_server_id, user_id) \
         VALUES ($1, $2, $3)",
    )
    .bind(project)
    .bind(server)
    .bind(owner.uuid())
    .execute(store.pool())
    .await
    .unwrap();

    let read = ProjectRepository::get(&store, owner, ProjectId::new(project))
        .await
        .unwrap()
        .expect("the owner reads it");
    assert_eq!(read.id(), ProjectId::new(project));
    assert_eq!(read.user_id(), owner);
    assert_eq!(read.name(), "Research");
    assert_eq!(read.icon(), "R");
    assert_eq!(read.instructions(), "Cite sources.");
    assert_eq!(read.default_model_id(), Some(default_model.id()));
    assert_eq!(read.reasoning(), Some(ReasoningLevel::High));
    assert_eq!(read.builtin_tools(), [BuiltinTool::Schedules]);
    assert_eq!(read.tool_server_ids(), [ToolServerId::new(server)]);
    assert!(read.pinned());
    assert_eq!(read.created_at(), at(0));
    assert_eq!(read.updated_at(), at(5));

    assert_eq!(
        ProjectRepository::get(&store, other, ProjectId::new(project))
            .await
            .unwrap(),
        None,
        "another user's project reads as missing"
    );
    assert_eq!(
        ProjectRepository::get(&store, owner, ProjectId::generate())
            .await
            .unwrap(),
        None
    );
}
