//! Project use cases.

use std::sync::Arc;

use crate::domains::project::entities::Project;
use crate::domains::project::ports::{ProjectQuery, ProjectRepository};
use crate::domains::provider::ModelRepository;
use crate::domains::tool::ToolServerRepository;
use crate::ports::clock::Clock;
use crate::shared::actor::Actor;
use crate::shared::builtin_tool::BuiltinTool;
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{ModelId, ProjectId, ToolServerId};
use crate::shared::reasoning::ReasoningLevel;

/// Input of [`ProjectService::create`].
#[derive(Clone, Debug, Default)]
pub struct NewProject {
    pub name: String,
    pub icon: String,
    pub instructions: String,
    pub default_model_id: Option<ModelId>,
    pub reasoning: Option<ReasoningLevel>,
    pub builtin_tools: Vec<BuiltinTool>,
    pub tool_server_ids: Vec<ToolServerId>,
    pub pinned: bool,
}

/// Input of [`ProjectService::update`]; `None` keeps a field, and the inner `Option` clears the
/// default model or the reasoning level.
#[derive(Clone, Debug, Default)]
pub struct ProjectUpdate {
    pub name: Option<String>,
    pub icon: Option<String>,
    pub instructions: Option<String>,
    pub default_model_id: Option<Option<ModelId>>,
    pub reasoning: Option<Option<ReasoningLevel>>,
    pub builtin_tools: Option<Vec<BuiltinTool>>,
    /// Replaces the project's tool servers.
    pub tool_server_ids: Option<Vec<ToolServerId>>,
    pub pinned: Option<bool>,
}

#[derive(Clone)]
pub struct ProjectService {
    projects: Arc<dyn ProjectRepository>,
    models: Arc<dyn ModelRepository>,
    servers: Arc<dyn ToolServerRepository>,
    clock: Arc<dyn Clock>,
}

impl ProjectService {
    pub fn new(
        projects: Arc<dyn ProjectRepository>,
        models: Arc<dyn ModelRepository>,
        servers: Arc<dyn ToolServerRepository>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            projects,
            models,
            servers,
            clock,
        }
    }

    /// Most recently updated first.
    pub async fn list(&self, actor: &Actor, query: ProjectQuery) -> DomainResult<Vec<Project>> {
        self.projects.list(actor.user_id, query).await
    }

    pub async fn get(&self, actor: &Actor, id: ProjectId) -> DomainResult<Project> {
        self.projects
            .get(actor.user_id, id)
            .await?
            .ok_or_else(|| not_found(id))
    }

    pub async fn create(&self, actor: &Actor, input: NewProject) -> DomainResult<Project> {
        self.check_model(actor, input.default_model_id).await?;
        self.check_servers(actor, &input.tool_server_ids).await?;
        let project = Project::create(actor.user_id, &input.name, self.clock.now())?
            .with_icon(&input.icon)?
            .with_instructions(&input.instructions)
            .with_default_model(input.default_model_id)
            .with_reasoning(input.reasoning)
            .with_builtin_tools(input.builtin_tools)
            .with_tool_servers(input.tool_server_ids)
            .with_pinned(input.pinned);
        self.projects.insert(&project).await?;
        Ok(project)
    }

    pub async fn update(
        &self,
        actor: &Actor,
        id: ProjectId,
        changes: ProjectUpdate,
    ) -> DomainResult<Project> {
        let mut project = self.get(actor, id).await?;
        if let Some(name) = &changes.name {
            project = project.with_name(name)?;
        }
        if let Some(icon) = &changes.icon {
            project = project.with_icon(icon)?;
        }
        if let Some(instructions) = &changes.instructions {
            project = project.with_instructions(instructions);
        }
        if let Some(model) = changes.default_model_id {
            self.check_model(actor, model).await?;
            project = project.with_default_model(model);
        }
        if let Some(reasoning) = changes.reasoning {
            project = project.with_reasoning(reasoning);
        }
        if let Some(tools) = changes.builtin_tools {
            project = project.with_builtin_tools(tools);
        }
        if let Some(servers) = changes.tool_server_ids {
            self.check_servers(actor, &servers).await?;
            project = project.with_tool_servers(servers);
        }
        if let Some(pinned) = changes.pinned {
            project = project.with_pinned(pinned);
        }
        let project = project.touched(self.clock.now());
        self.projects.update(&project).await?;
        Ok(project)
    }

    /// Deletes the project with its chats, files, and schedules. The API handler cancels live
    /// runs of the project's chats before calling this.
    pub async fn delete(&self, actor: &Actor, id: ProjectId) -> DomainResult<()> {
        if self.projects.delete(actor.user_id, id).await? {
            Ok(())
        } else {
            Err(not_found(id))
        }
    }

    /// A default model must be one of the caller's models.
    async fn check_model(&self, actor: &Actor, model: Option<ModelId>) -> DomainResult<()> {
        let Some(id) = model else {
            return Ok(());
        };
        match self.models.get(actor.user_id, id).await? {
            Some(_) => Ok(()),
            None => Err(DomainError::not_found("Model", id.to_string())),
        }
    }

    /// Every tool server must be one of the caller's servers.
    async fn check_servers(&self, actor: &Actor, ids: &[ToolServerId]) -> DomainResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let owned = self.servers.list(actor.user_id).await?;
        match ids
            .iter()
            .find(|id| !owned.iter().any(|server| server.id() == **id))
        {
            Some(id) => Err(DomainError::not_found("Tool server", id.to_string())),
            None => Ok(()),
        }
    }
}

fn not_found(id: ProjectId) -> DomainError {
    DomainError::not_found("Project", id.to_string())
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::domains::provider::{
        ModelQuery, NewProvider, Preset, ProviderKind, ProviderService,
    };
    use crate::domains::tool::{NewToolServer, ToolService};
    use crate::shared::ids::ProviderId;
    use crate::shared::net::PrivateNetwork;
    use crate::testing::{
        FakeCipher, FixedClock, MemoryStore, ScriptedGateway, ScriptedToolHost, actor, remote_model,
    };

    struct Fixture {
        service: ProjectService,
        providers: ProviderService,
        tools: ToolService,
        gateway: Arc<ScriptedGateway>,
        store: Arc<MemoryStore>,
        clock: Arc<FixedClock>,
    }

    fn fixture() -> Fixture {
        let clock = Arc::new(FixedClock::default());
        let store = Arc::new(MemoryStore::new(clock.clone()));
        let gateway = Arc::new(ScriptedGateway::new());
        let providers = ProviderService::new(
            store.clone(),
            store.clone(),
            store.clone(),
            gateway.clone(),
            Arc::new(FakeCipher::new()),
            clock.clone(),
            PrivateNetwork::Admins,
        );
        let tools = ToolService::new(
            store.clone(),
            Arc::new(ScriptedToolHost::new()),
            Arc::new(FakeCipher::new()),
            clock.clone(),
            PrivateNetwork::Admins,
        );
        Fixture {
            service: ProjectService::new(
                store.clone(),
                store.clone(),
                store.clone(),
                clock.clone(),
            ),
            providers,
            tools,
            gateway,
            store,
            clock,
        }
    }

    /// A connection with one model; returns the model id and the provider id.
    async fn model(f: &Fixture, caller: &Actor) -> (ModelId, ProviderId) {
        let provider = f
            .providers
            .create(
                caller,
                NewProvider {
                    kind: ProviderKind::OpenAi,
                    preset: Preset::OpenAi,
                    name: "OpenAI".to_owned(),
                    base_url: None,
                    api_key: Some("sk".to_owned()),
                },
            )
            .await
            .unwrap();
        f.gateway.push_models(vec![remote_model("gpt-5.6-terra")]);
        f.providers.refresh(caller, provider.id()).await.unwrap();
        let models = ModelRepository::list(
            f.store.as_ref(),
            caller.user_id,
            ModelQuery::provider(provider.id()),
        )
        .await
        .unwrap();
        (models[0].id(), provider.id())
    }

    async fn server(f: &Fixture, caller: &Actor) -> ToolServerId {
        let input = NewToolServer {
            name: "GitHub".to_owned(),
            url: "https://mcp.example.com/mcp".to_owned(),
            ..NewToolServer::default()
        };
        f.tools.create(caller, input).await.unwrap().id()
    }

    fn named(name: &str) -> NewProject {
        NewProject {
            name: name.to_owned(),
            ..NewProject::default()
        }
    }

    #[tokio::test]
    async fn creates_a_project_with_every_setting() {
        let f = fixture();
        let caller = actor();
        let (model, _) = model(&f, &caller).await;
        let server = server(&f, &caller).await;
        let input = NewProject {
            name: " Contracts ".to_owned(),
            icon: "📄".to_owned(),
            instructions: " You review contracts. ".to_owned(),
            default_model_id: Some(model),
            reasoning: Some(ReasoningLevel::High),
            builtin_tools: vec![BuiltinTool::Schedules, BuiltinTool::Schedules],
            tool_server_ids: vec![server, server],
            pinned: true,
        };
        let project = f.service.create(&caller, input).await.unwrap();
        assert_eq!(project.name(), "Contracts");
        assert_eq!(project.icon(), "📄");
        assert_eq!(project.instructions(), "You review contracts.");
        assert_eq!(project.default_model_id(), Some(model));
        assert_eq!(project.reasoning(), Some(ReasoningLevel::High));
        assert_eq!(project.builtin_tools(), [BuiltinTool::Schedules]);
        assert_eq!(project.tool_server_ids(), [server]);
        assert!(project.pinned());
        assert_eq!(f.service.get(&caller, project.id()).await.unwrap(), project);
    }

    #[tokio::test]
    async fn validates_name_and_icon() {
        let f = fixture();
        let error = f.service.create(&actor(), named("  ")).await.unwrap_err();
        assert_eq!(error.to_string(), "Project name is required.");
        let input = NewProject {
            icon: "folder".to_owned(),
            ..named("Work")
        };
        let error = f.service.create(&actor(), input).await.unwrap_err();
        assert_eq!(error.to_string(), "Choose one emoji as the icon.");
    }

    #[tokio::test]
    async fn rejects_another_users_model() {
        let f = fixture();
        let alice = actor();
        let bob = actor();
        let (model, _) = model(&f, &alice).await;
        let input = NewProject {
            default_model_id: Some(model),
            ..named("Work")
        };
        let error = f.service.create(&bob, input).await.unwrap_err();
        assert!(
            matches!(error, DomainError::EntityNotFound { ref entity_name, .. } if entity_name == "Model")
        );

        let project = f.service.create(&bob, named("Work")).await.unwrap();
        let changes = ProjectUpdate {
            default_model_id: Some(Some(model)),
            ..ProjectUpdate::default()
        };
        let error = f
            .service
            .update(&bob, project.id(), changes)
            .await
            .unwrap_err();
        assert!(matches!(error, DomainError::EntityNotFound { .. }));
    }

    #[tokio::test]
    async fn rejects_another_users_tool_server() {
        let f = fixture();
        let alice = actor();
        let bob = actor();
        let server = server(&f, &alice).await;
        let input = NewProject {
            tool_server_ids: vec![server],
            ..named("Work")
        };
        let error = f.service.create(&bob, input).await.unwrap_err();
        assert!(
            matches!(error, DomainError::EntityNotFound { ref entity_name, .. } if entity_name == "Tool server")
        );

        let project = f.service.create(&bob, named("Work")).await.unwrap();
        let changes = ProjectUpdate {
            tool_server_ids: Some(vec![server]),
            ..ProjectUpdate::default()
        };
        let error = f
            .service
            .update(&bob, project.id(), changes)
            .await
            .unwrap_err();
        assert!(matches!(error, DomainError::EntityNotFound { .. }));
        let unknown = ProjectUpdate {
            tool_server_ids: Some(vec![ToolServerId::generate()]),
            ..ProjectUpdate::default()
        };
        assert!(f.service.update(&bob, project.id(), unknown).await.is_err());
    }

    #[tokio::test]
    async fn deleting_a_tool_server_removes_it_from_the_project() {
        let f = fixture();
        let caller = actor();
        let kept = server(&f, &caller).await;
        let removed = server(&f, &caller).await;
        let input = NewProject {
            tool_server_ids: vec![removed, kept],
            ..named("Work")
        };
        let project = f.service.create(&caller, input).await.unwrap();
        f.tools.delete(&caller, removed).await.unwrap();
        let project = f.service.get(&caller, project.id()).await.unwrap();
        assert_eq!(project.tool_server_ids(), [kept]);
    }

    #[tokio::test]
    async fn updates_fields_and_clears_options() {
        let f = fixture();
        let caller = actor();
        let (model, _) = model(&f, &caller).await;
        let input = NewProject {
            default_model_id: Some(model),
            reasoning: Some(ReasoningLevel::Low),
            ..named("Work")
        };
        let project = f.service.create(&caller, input).await.unwrap();
        f.clock.advance(Duration::minutes(1));
        let changes = ProjectUpdate {
            name: Some("Research".to_owned()),
            icon: Some("🔬".to_owned()),
            instructions: Some("Cite sources.".to_owned()),
            default_model_id: Some(None),
            reasoning: Some(None),
            builtin_tools: Some(vec![BuiltinTool::Schedules]),
            tool_server_ids: Some(vec![]),
            pinned: Some(true),
        };
        let updated = f
            .service
            .update(&caller, project.id(), changes)
            .await
            .unwrap();
        assert_eq!(updated.name(), "Research");
        assert_eq!(updated.icon(), "🔬");
        assert_eq!(updated.instructions(), "Cite sources.");
        assert_eq!(updated.default_model_id(), None);
        assert_eq!(updated.reasoning(), None);
        assert_eq!(updated.builtin_tools(), [BuiltinTool::Schedules]);
        assert!(updated.pinned());
        assert_eq!(updated.created_at(), project.created_at());
        assert_eq!(updated.updated_at(), f.clock.now());
        assert_eq!(f.service.get(&caller, project.id()).await.unwrap(), updated);
    }

    #[tokio::test]
    async fn lists_recent_first_with_pinned_filter_and_limit() {
        let f = fixture();
        let caller = actor();
        let first = f.service.create(&caller, named("First")).await.unwrap();
        f.clock.advance(Duration::minutes(1));
        let second = f.service.create(&caller, named("Second")).await.unwrap();
        f.clock.advance(Duration::minutes(1));
        let pin = ProjectUpdate {
            pinned: Some(true),
            ..ProjectUpdate::default()
        };
        let first = f.service.update(&caller, first.id(), pin).await.unwrap();

        let all = f
            .service
            .list(&caller, ProjectQuery::default())
            .await
            .unwrap();
        assert_eq!(all, [first.clone(), second.clone()]);
        let pinned = ProjectQuery {
            pinned: Some(true),
            limit: None,
        };
        assert_eq!(f.service.list(&caller, pinned).await.unwrap(), [first]);
        let recent = ProjectQuery {
            pinned: None,
            limit: Some(1),
        };
        assert_eq!(f.service.list(&caller, recent).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn hides_other_users_projects() {
        let f = fixture();
        let alice = actor();
        let bob = actor();
        let project = f.service.create(&alice, named("Work")).await.unwrap();
        assert!(
            f.service
                .list(&bob, ProjectQuery::default())
                .await
                .unwrap()
                .is_empty()
        );
        for error in [
            f.service.get(&bob, project.id()).await.unwrap_err(),
            f.service
                .update(&bob, project.id(), ProjectUpdate::default())
                .await
                .unwrap_err(),
            f.service.delete(&bob, project.id()).await.unwrap_err(),
        ] {
            assert!(
                matches!(error, DomainError::EntityNotFound { .. }),
                "{error}"
            );
        }
        assert!(f.service.get(&alice, project.id()).await.is_ok());
    }

    #[tokio::test]
    async fn deletes_a_project() {
        let f = fixture();
        let caller = actor();
        let project = f.service.create(&caller, named("Work")).await.unwrap();
        f.service.delete(&caller, project.id()).await.unwrap();
        let error = f.service.get(&caller, project.id()).await.unwrap_err();
        assert!(matches!(error, DomainError::EntityNotFound { .. }));
    }

    #[tokio::test]
    async fn deleting_a_provider_clears_the_default_model_and_counts_it_first() {
        let f = fixture();
        let caller = actor();
        let (model, provider) = model(&f, &caller).await;
        let input = NewProject {
            default_model_id: Some(model),
            ..named("Work")
        };
        let project = f.service.create(&caller, input).await.unwrap();
        let details = f.providers.get(&caller, provider).await.unwrap();
        assert_eq!(details.usage.projects, 1);
        f.providers.delete(&caller, provider).await.unwrap();
        let project = f.service.get(&caller, project.id()).await.unwrap();
        assert_eq!(project.default_model_id(), None, "ON DELETE SET NULL");
    }
}
