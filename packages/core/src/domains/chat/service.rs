//! Chat use cases: settings, read state, search, save as project, startup recovery, and turn
//! preparation (`prepare`, in the submodule).

#[cfg(test)]
mod fixture;
mod prepare;

use std::sync::Arc;

use crate::domains::chat::entities::Chat;
use crate::domains::chat::message::Message;
use crate::domains::chat::ports::{
    ChatQuery, ChatRepository, ChatSummary, MessageRepository, Page, SearchHit,
    TurnContextRepository,
};
use crate::domains::file::FileRepository;
use crate::domains::project::{Project, ProjectRepository};
use crate::domains::provider::{ModelRepository, ProviderRepository};
use crate::domains::schedule::ScheduleRepository;
use crate::domains::settings::SettingsRepository;
use crate::domains::tool::{ServerOverride, ToolServerRepository, ToolService};
use crate::ports::clock::Clock;
use crate::ports::secret_cipher::SecretCipher;
use crate::shared::actor::Actor;
use crate::shared::builtin_tool::BuiltinTool;
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{ChatId, ModelId, ProjectId, ScheduleId};
use crate::shared::net::PrivateNetwork;
use crate::shared::reasoning::ReasoningLevel;

/// Largest page of the chat list.
const PAGE_MAX: u32 = 100;
/// Search results per request at most.
const SEARCH_LIMIT_MAX: u32 = 50;
/// Longest search text, in characters.
const SEARCH_MAX_CHARS: usize = 200;
/// Name of a project saved from an untitled chat.
const UNTITLED_PROJECT: &str = "New project";

/// The repositories a [`ChatService`] reads and writes.
#[derive(Clone)]
pub struct ChatRepositories {
    pub chats: Arc<dyn ChatRepository>,
    pub messages: Arc<dyn MessageRepository>,
    pub contexts: Arc<dyn TurnContextRepository>,
    pub settings: Arc<dyn SettingsRepository>,
    pub projects: Arc<dyn ProjectRepository>,
    pub models: Arc<dyn ModelRepository>,
    pub providers: Arc<dyn ProviderRepository>,
    pub files: Arc<dyn FileRepository>,
    pub schedules: Arc<dyn ScheduleRepository>,
    pub servers: Arc<dyn ToolServerRepository>,
}

impl ChatRepositories {
    /// Every repository from one store that implements them all (the database adapter, or
    /// `testing::MemoryStore`).
    pub fn shared<S>(store: Arc<S>) -> Self
    where
        S: ChatRepository
            + MessageRepository
            + TurnContextRepository
            + SettingsRepository
            + ProjectRepository
            + ModelRepository
            + ProviderRepository
            + FileRepository
            + ScheduleRepository
            + ToolServerRepository
            + 'static,
    {
        Self {
            chats: store.clone(),
            messages: store.clone(),
            contexts: store.clone(),
            settings: store.clone(),
            projects: store.clone(),
            models: store.clone(),
            providers: store.clone(),
            files: store.clone(),
            schedules: store.clone(),
            servers: store,
        }
    }
}

/// Input of [`ChatService::create`] (the new chat draft). The title comes from the first user
/// message.
#[derive(Clone, Debug, Default)]
pub struct NewChat {
    pub project_id: Option<ProjectId>,
    pub model_id: Option<ModelId>,
    pub instructions: String,
    pub reasoning: Option<ReasoningLevel>,
    /// `None` inherits the project's.
    pub builtin_tools: Option<Vec<BuiltinTool>>,
    pub tool_overrides: Vec<ServerOverride>,
    pub voice_replies: bool,
}

/// Input of [`ChatService::update`]; `None` keeps a field, and an inner `None` clears it.
#[derive(Clone, Debug, Default)]
pub struct ChatUpdate {
    pub title: Option<String>,
    /// Move to a project, or out of one.
    pub project_id: Option<Option<ProjectId>>,
    pub model_id: Option<Option<ModelId>>,
    pub instructions: Option<String>,
    pub reasoning: Option<Option<ReasoningLevel>>,
    pub builtin_tools: Option<Option<Vec<BuiltinTool>>>,
    /// Replaces every override.
    pub tool_overrides: Option<Vec<ServerOverride>>,
    pub pinned: Option<bool>,
    pub voice_replies: Option<bool>,
}

#[derive(Clone)]
pub struct ChatService {
    stores: ChatRepositories,
    tools: ToolService,
    cipher: Arc<dyn SecretCipher>,
    clock: Arc<dyn Clock>,
    network: PrivateNetwork,
}

impl ChatService {
    /// `network` is `ALLOW_PRIVATE_NETWORK`.
    pub fn new(
        stores: ChatRepositories,
        tools: ToolService,
        cipher: Arc<dyn SecretCipher>,
        clock: Arc<dyn Clock>,
        network: PrivateNetwork,
    ) -> Self {
        Self {
            stores,
            tools,
            cipher,
            clock,
            network,
        }
    }

    /// The sidebar lists, most recently updated first, with unread and awaiting-approval flags.
    pub async fn list(
        &self,
        actor: &Actor,
        mut query: ChatQuery,
    ) -> DomainResult<Page<ChatSummary>> {
        if let Some(project) = query.project {
            self.check_project(actor, project).await?;
        }
        if let Some(schedule) = query.schedule {
            self.check_schedule(actor, schedule).await?;
        }
        query.limit = query.limit.clamp(1, PAGE_MAX);
        self.stores.chats.list(actor.user_id, query).await
    }

    pub async fn get(&self, actor: &Actor, id: ChatId) -> DomainResult<Chat> {
        self.stores
            .chats
            .get(actor.user_id, id)
            .await?
            .ok_or_else(|| not_found(id))
    }

    /// The chat's messages, in order.
    pub async fn messages(&self, actor: &Actor, id: ChatId) -> DomainResult<Vec<Message>> {
        self.get(actor, id).await?;
        self.stores.messages.history(actor.user_id, id).await
    }

    /// Creates the chat of a new chat draft on its first send, so empty chats never exist.
    pub async fn create(&self, actor: &Actor, input: NewChat) -> DomainResult<Chat> {
        if let Some(project) = input.project_id {
            self.check_project(actor, project).await?;
        }
        self.check_model(actor, input.model_id).await?;
        self.check_servers(actor, &input.tool_overrides).await?;
        let chat = Chat::create(actor.user_id, self.clock.now())
            .with_project(input.project_id)
            .with_model(input.model_id)
            .with_instructions(&input.instructions)
            .with_reasoning(input.reasoning)
            .with_builtin_tools(input.builtin_tools)
            .with_tool_overrides(input.tool_overrides)
            .with_voice_replies(input.voice_replies);
        self.stores.chats.insert(&chat).await?;
        Ok(chat)
    }

    /// Renames, moves, or changes the chat's settings. It does not count as a change for the
    /// unread dot or the recent list.
    pub async fn update(
        &self,
        actor: &Actor,
        id: ChatId,
        changes: ChatUpdate,
    ) -> DomainResult<Chat> {
        let mut chat = self.get(actor, id).await?;
        if let Some(title) = &changes.title {
            chat = chat.with_title(title)?;
        }
        if let Some(project) = changes.project_id {
            if let Some(project) = project {
                self.check_project(actor, project).await?;
            }
            chat = chat.with_project(project);
        }
        if let Some(model) = changes.model_id {
            self.check_model(actor, model).await?;
            chat = chat.with_model(model);
        }
        if let Some(instructions) = &changes.instructions {
            chat = chat.with_instructions(instructions);
        }
        if let Some(reasoning) = changes.reasoning {
            chat = chat.with_reasoning(reasoning);
        }
        if let Some(tools) = changes.builtin_tools {
            chat = chat.with_builtin_tools(tools);
        }
        if let Some(overrides) = changes.tool_overrides {
            self.check_servers(actor, &overrides).await?;
            chat = chat.with_tool_overrides(overrides);
        }
        if let Some(pinned) = changes.pinned {
            chat = chat.with_pinned(pinned);
        }
        if let Some(voice_replies) = changes.voice_replies {
            chat = chat.with_voice_replies(voice_replies);
        }
        self.stores.chats.update(&chat).await?;
        Ok(chat)
    }

    /// Deletes the chat with its messages and attachments. The API handler cancels a live run
    /// of the chat first.
    pub async fn delete(&self, actor: &Actor, id: ChatId) -> DomainResult<()> {
        if self.stores.chats.delete(actor.user_id, id).await? {
            Ok(())
        } else {
            Err(not_found(id))
        }
    }

    /// The user opened the chat: it is no longer unread.
    pub async fn mark_read(&self, actor: &Actor, id: ChatId) -> DomainResult<()> {
        self.get(actor, id).await?;
        self.stores
            .chats
            .mark_read(actor.user_id, id, self.clock.now())
            .await
    }

    /// Chat titles and message text matching `text`.
    pub async fn search(
        &self,
        actor: &Actor,
        text: &str,
        limit: u32,
    ) -> DomainResult<Vec<SearchHit>> {
        let text = text.trim();
        if text.is_empty() {
            return Err(DomainError::validation("Enter text to search for.").with_field("q"));
        }
        if text.chars().count() > SEARCH_MAX_CHARS {
            return Err(DomainError::validation(format!(
                "Search text must be at most {SEARCH_MAX_CHARS} characters."
            ))
            .with_field("q"));
        }
        self.stores
            .messages
            .search(actor.user_id, text, limit.clamp(1, SEARCH_LIMIT_MAX))
            .await
    }

    /// Save as project, in one transaction: a project from the standalone chat (name = title,
    /// instructions, default model, reasoning, enabled tool servers, and built-in tools), the
    /// chat's attachments moved into it as project files, the chat moved into it, and the chat's
    /// instructions and tool overrides cleared so nothing is sent twice. Of two saves of the same
    /// chat at once, one creates the project and the other fails with `CHAT_IN_PROJECT`.
    pub async fn save_as_project(&self, actor: &Actor, id: ChatId) -> DomainResult<Project> {
        let chat = self.get(actor, id).await?;
        if chat.project_id().is_some() {
            return Err(in_project());
        }
        let name = if chat.title().is_empty() {
            UNTITLED_PROJECT
        } else {
            chat.title()
        };
        let servers = chat
            .tool_overrides()
            .iter()
            .filter(|row| row.enabled)
            .map(|row| row.server_id)
            .collect();
        let project = Project::create(actor.user_id, name, self.clock.now())?
            .with_instructions(chat.instructions())
            .with_default_model(chat.model_id())
            .with_reasoning(chat.reasoning())
            .with_builtin_tools(chat.builtin_tools().map(<[_]>::to_vec).unwrap_or_default())
            .with_tool_servers(servers);
        if !self.stores.projects.insert_with_chat(&project, id).await? {
            // Saved or moved by another request since it was read, or deleted (then `404`).
            self.get(actor, id).await?;
            return Err(in_project());
        }
        Ok(project)
    }

    /// Save instructions to project: appends the chat's instructions to its project's (after a
    /// blank line) and clears them on the chat.
    pub async fn save_instructions(&self, actor: &Actor, id: ChatId) -> DomainResult<Project> {
        let chat = self.get(actor, id).await?;
        let Some(project_id) = chat.project_id() else {
            return Err(DomainError::business_rule(
                "This chat is not in a project.",
                "CHAT_NOT_IN_PROJECT",
            ));
        };
        if chat.instructions().is_empty() {
            return Err(
                DomainError::validation("This chat has no instructions to save.")
                    .with_field("instructions"),
            );
        }
        let project = self
            .stores
            .projects
            .get(actor.user_id, project_id)
            .await?
            .ok_or_else(|| DomainError::not_found("Project", project_id.to_string()))?;
        let combined = if project.instructions().is_empty() {
            chat.instructions().to_owned()
        } else {
            format!("{}\n\n{}", project.instructions(), chat.instructions())
        };
        let project = project
            .with_instructions(&combined)
            .touched(self.clock.now());
        self.stores.projects.update(&project).await?;
        self.stores
            .chats
            .update(&chat.with_instructions(""))
            .await?;
        Ok(project)
    }

    /// At startup, before serving: messages that were streaming when the server stopped become
    /// errors ("Interrupted by a server restart."). Messages awaiting approval stay valid.
    pub async fn recover(&self) -> DomainResult<u64> {
        self.stores.messages.recover(self.clock.now()).await
    }

    async fn check_project(&self, actor: &Actor, project: ProjectId) -> DomainResult<()> {
        self.stores
            .projects
            .get(actor.user_id, project)
            .await?
            .map(|_| ())
            .ok_or_else(|| DomainError::not_found("Project", project.to_string()))
    }

    async fn check_schedule(&self, actor: &Actor, schedule: ScheduleId) -> DomainResult<()> {
        self.stores
            .schedules
            .get(actor.user_id, schedule)
            .await?
            .map(|_| ())
            .ok_or_else(|| DomainError::not_found("Schedule", schedule.to_string()))
    }

    async fn check_model(&self, actor: &Actor, model: Option<ModelId>) -> DomainResult<()> {
        let Some(id) = model else {
            return Ok(());
        };
        self.stores
            .models
            .get(actor.user_id, id)
            .await?
            .map(|_| ())
            .ok_or_else(|| DomainError::not_found("Model", id.to_string()))
    }

    async fn check_servers(&self, actor: &Actor, overrides: &[ServerOverride]) -> DomainResult<()> {
        if overrides.is_empty() {
            return Ok(());
        }
        let owned = self.stores.servers.list(actor.user_id).await?;
        match overrides
            .iter()
            .find(|row| !owned.iter().any(|server| server.id() == row.server_id))
        {
            Some(row) => Err(DomainError::not_found(
                "Tool server",
                row.server_id.to_string(),
            )),
            None => Ok(()),
        }
    }
}

fn not_found(id: ChatId) -> DomainError {
    DomainError::not_found("Chat", id.to_string())
}

fn in_project() -> DomainError {
    DomainError::business_rule("This chat is already in a project.", "CHAT_IN_PROJECT")
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use chrono::Duration;

    use super::*;
    use crate::domains::chat::message::{INTERRUPTED, MessageParts, MessageStatus, Part};
    use crate::domains::chat::ports::{ChatCursor, MessageRepository};
    use crate::domains::chat::service::fixture::{self, chat, fixture, model, pause, send, server};
    use crate::domains::file::FileOwner;
    use crate::domains::project::{NewProject, ProjectQuery};
    use crate::domains::schedule::{NewSchedule, SchedulePrompt, ScheduleQuery};
    use crate::domains::tool::ToolSelection;
    use crate::shared::ids::{ProviderId, UserId};
    use crate::testing::actor;

    fn is_not_found(error: &DomainError, entity: &str) -> bool {
        matches!(error, DomainError::EntityNotFound { entity_name, .. } if entity_name == entity)
    }

    async fn project(f: &fixture::Fixture, caller: &Actor, name: &str) -> Project {
        let input = NewProject {
            name: name.to_owned(),
            ..NewProject::default()
        };
        f.projects.create(caller, input).await.unwrap()
    }

    #[tokio::test]
    async fn creates_a_chat_from_the_draft() {
        let f = fixture();
        let caller = actor();
        let (model, _) = model(&f, &caller).await;
        let project = project(&f, &caller, "Work").await;
        let server = server(&f, &caller).await;
        let input = NewChat {
            project_id: Some(project.id()),
            model_id: Some(model),
            instructions: " Be brief. ".to_owned(),
            reasoning: Some(ReasoningLevel::Low),
            builtin_tools: Some(vec![BuiltinTool::Schedules]),
            tool_overrides: vec![ServerOverride {
                server_id: server,
                enabled: true,
            }],
            voice_replies: true,
        };
        let chat = f.chats.create(&caller, input).await.unwrap();
        assert_eq!(chat.title(), "", "titled by the first message");
        assert_eq!(chat.project_id(), Some(project.id()));
        assert_eq!(chat.model_id(), Some(model));
        assert_eq!(chat.instructions(), "Be brief.");
        assert_eq!(chat.builtin_tools(), Some(&[BuiltinTool::Schedules][..]));
        assert!(chat.voice_replies());
        assert!(!chat.unread());
        assert_eq!(f.chats.get(&caller, chat.id()).await.unwrap(), chat);
    }

    #[tokio::test]
    async fn rejects_other_users_references() {
        let f = fixture();
        let alice = actor();
        let bob = actor();
        let (model, _) = model(&f, &alice).await;
        let project = project(&f, &alice, "Work").await;
        let server = server(&f, &alice).await;
        let cases = [
            (
                NewChat {
                    model_id: Some(model),
                    ..NewChat::default()
                },
                "Model",
            ),
            (
                NewChat {
                    project_id: Some(project.id()),
                    ..NewChat::default()
                },
                "Project",
            ),
            (
                NewChat {
                    tool_overrides: vec![ServerOverride {
                        server_id: server,
                        enabled: true,
                    }],
                    ..NewChat::default()
                },
                "Tool server",
            ),
        ];
        for (input, entity) in cases {
            let error = f.chats.create(&bob, input).await.unwrap_err();
            assert!(is_not_found(&error, entity), "{error}");
        }
        let chat = f.chats.create(&bob, NewChat::default()).await.unwrap();
        let moved = ChatUpdate {
            project_id: Some(Some(project.id())),
            ..ChatUpdate::default()
        };
        let error = f.chats.update(&bob, chat.id(), moved).await.unwrap_err();
        assert!(is_not_found(&error, "Project"));
    }

    #[tokio::test]
    async fn settings_changes_do_not_mark_the_chat_unread() {
        let f = fixture();
        let caller = actor();
        let chat = chat(&f, &caller).await;
        let project = project(&f, &caller, "Work").await;
        f.clock.advance(Duration::minutes(5));
        let changes = ChatUpdate {
            title: Some("Trip".to_owned()),
            project_id: Some(Some(project.id())),
            model_id: Some(None),
            instructions: Some("Plan it.".to_owned()),
            reasoning: Some(Some(ReasoningLevel::High)),
            builtin_tools: Some(Some(vec![BuiltinTool::Schedules])),
            tool_overrides: Some(Vec::new()),
            pinned: Some(true),
            voice_replies: Some(true),
        };
        let updated = f.chats.update(&caller, chat.id(), changes).await.unwrap();
        assert_eq!(updated.title(), "Trip");
        assert_eq!(updated.project_id(), Some(project.id()));
        assert_eq!(updated.model_id(), None);
        assert!(updated.pinned());
        let stored = f.chats.get(&caller, chat.id()).await.unwrap();
        assert_eq!(stored.updated_at(), chat.updated_at());
        assert!(!stored.unread());
        let error = f
            .chats
            .update(
                &caller,
                chat.id(),
                ChatUpdate {
                    title: Some(" ".to_owned()),
                    ..ChatUpdate::default()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "Title is required.");
    }

    #[tokio::test]
    async fn lists_recent_first_with_filters_flags_and_cursor() {
        let f = fixture();
        let caller = actor();
        let first = chat(&f, &caller).await;
        f.clock.advance(Duration::minutes(1));
        let project = project(&f, &caller, "Work").await;
        let second = f
            .chats
            .create(
                &caller,
                NewChat {
                    project_id: Some(project.id()),
                    model_id: first.model_id(),
                    ..NewChat::default()
                },
            )
            .await
            .unwrap();
        f.clock.advance(Duration::minutes(1));
        let third = f
            .chats
            .create(
                &caller,
                NewChat {
                    model_id: first.model_id(),
                    ..NewChat::default()
                },
            )
            .await
            .unwrap();
        f.clock.advance(Duration::minutes(1));
        pause(&f, &caller, &second).await;
        f.clock.advance(Duration::minutes(1));
        f.chats
            .prepare(&caller, first.id(), send("Hello"))
            .await
            .unwrap();

        let page = f.chats.list(&caller, ChatQuery::default()).await.unwrap();
        let ids: Vec<ChatId> = page.items.iter().map(|row| row.chat.id()).collect();
        assert_eq!(
            ids,
            [first.id(), second.id(), third.id()],
            "most recent first"
        );
        assert!(page.next.is_none());
        assert!(
            page.items[0].unread,
            "the new message counts until it is read"
        );
        assert!(page.items[1].awaiting_approval);
        assert!(!page.items[2].unread);
        assert!(!page.items[0].awaiting_approval);

        let first_page = f
            .chats
            .list(
                &caller,
                ChatQuery {
                    limit: 2,
                    ..ChatQuery::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(first_page.items.len(), 2);
        let cursor = first_page.next.expect("more chats follow");
        assert_eq!(
            cursor,
            ChatCursor {
                updated_at: first_page.items[1].chat.updated_at(),
                id: second.id(),
            }
        );
        let rest = f
            .chats
            .list(
                &caller,
                ChatQuery {
                    limit: 2,
                    cursor: Some(cursor),
                    ..ChatQuery::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(rest.items.len(), 1);
        assert_eq!(rest.items[0].chat.id(), third.id());

        let in_project = ChatQuery {
            project: Some(project.id()),
            ..ChatQuery::default()
        };
        let page = f.chats.list(&caller, in_project).await.unwrap();
        assert_eq!(page.items.len(), 1);
        let error = f.chats.list(&actor(), in_project).await.unwrap_err();
        assert!(is_not_found(&error, "Project"));

        f.chats.mark_read(&caller, first.id()).await.unwrap();
        let page = f.chats.list(&caller, ChatQuery::default()).await.unwrap();
        assert!(!page.items[0].unread);
    }

    #[tokio::test]
    async fn separates_schedule_runs_from_recent_chats() {
        let f = fixture();
        let caller = actor();
        let chat = chat(&f, &caller).await;
        let input = NewSchedule {
            name: "Digest".to_owned(),
            cron: "0 9 * * *".to_owned(),
            timezone: "UTC".to_owned(),
            prompt: "News?".to_owned(),
            model_id: chat.model_id(),
            ..NewSchedule::default()
        };
        let schedule = f.schedules.create(&caller, input).await.unwrap();
        let firing = f.schedules.run_now(&caller, schedule.id()).await.unwrap();
        let run = f
            .chats
            .prepare_scheduled(firing)
            .await
            .unwrap()
            .ready()
            .expect("the run starts")
            .chat;

        let scheduled = ChatQuery {
            scheduled: Some(true),
            ..ChatQuery::default()
        };
        let page = f.chats.list(&caller, scheduled).await.unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].chat.id(), run.id());
        assert!(page.items[0].unread, "an unopened run is unread");
        let recent = ChatQuery {
            scheduled: Some(false),
            ..ChatQuery::default()
        };
        let page = f.chats.list(&caller, recent).await.unwrap();
        assert_eq!(page.items[0].chat.id(), chat.id());
        let runs = ChatQuery {
            schedule: Some(schedule.id()),
            ..ChatQuery::default()
        };
        assert_eq!(f.chats.list(&caller, runs).await.unwrap().items.len(), 1);
        assert!(f.chats.list(&actor(), runs).await.is_err());

        f.schedules.delete(&caller, schedule.id()).await.unwrap();
        let kept = f.chats.get(&caller, run.id()).await.unwrap();
        assert_eq!(kept.schedule_id(), None, "the run's chat stays");
    }

    #[tokio::test]
    async fn hides_other_users_chats() {
        let f = fixture();
        let alice = actor();
        let bob = actor();
        let chat = chat(&f, &alice).await;
        let errors = [
            f.chats.get(&bob, chat.id()).await.unwrap_err(),
            f.chats.messages(&bob, chat.id()).await.unwrap_err(),
            f.chats
                .update(&bob, chat.id(), ChatUpdate::default())
                .await
                .unwrap_err(),
            f.chats.mark_read(&bob, chat.id()).await.unwrap_err(),
            f.chats
                .prepare(&bob, chat.id(), send("Hi"))
                .await
                .unwrap_err(),
            f.chats.save_as_project(&bob, chat.id()).await.unwrap_err(),
            f.chats
                .save_instructions(&bob, chat.id())
                .await
                .unwrap_err(),
            f.chats.delete(&bob, chat.id()).await.unwrap_err(),
        ];
        for error in errors {
            assert!(is_not_found(&error, "Chat"), "{error}");
        }
        assert!(
            f.chats
                .list(&bob, ChatQuery::default())
                .await
                .unwrap()
                .items
                .is_empty()
        );
        assert!(f.chats.get(&alice, chat.id()).await.is_ok());
    }

    #[tokio::test]
    async fn deleting_a_chat_removes_messages_and_attachments() {
        let f = fixture();
        let caller = actor();
        let chat = chat(&f, &caller).await;
        let file = f
            .files
            .upload(
                &caller,
                FileOwner::Chat(chat.id()),
                "notes.txt",
                Bytes::from_static(b"hello"),
            )
            .await
            .unwrap();
        f.chats
            .prepare(&caller, chat.id(), send("Hi"))
            .await
            .unwrap();
        let origin = chat.origin();
        let input = SchedulePrompt {
            name: "Digest".to_owned(),
            cron: "0 9 * * *".to_owned(),
            timezone: "UTC".to_owned(),
            prompt: "News?".to_owned(),
        };
        let schedule = f
            .schedules
            .schedule_prompt(&caller, &origin, input)
            .await
            .unwrap();

        f.chats.delete(&caller, chat.id()).await.unwrap();
        assert!(f.chats.get(&caller, chat.id()).await.is_err());
        assert!(
            f.store
                .history(caller.user_id, chat.id())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(f.files.get(&caller, file.id()).await.is_err());
        let kept = f.schedules.get(&caller, schedule.id()).await.unwrap();
        assert_eq!(kept.source_chat_id(), None, "ON DELETE SET NULL");
        let listed = f
            .schedules
            .list(&caller, ScheduleQuery::default())
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
    }

    #[tokio::test]
    async fn searches_titles_and_messages() {
        let f = fixture();
        let caller = actor();
        let chat = chat(&f, &caller).await;
        f.chats
            .prepare(&caller, chat.id(), send("Plan a trip to Lisbon"))
            .await
            .unwrap();
        let hits = f.chats.search(&caller, "  lisbon ", 10).await.unwrap();
        assert_eq!(hits.len(), 2, "the title and the message");
        assert_eq!(hits[0].message_id, None);
        assert_eq!(hits[0].chat_title, "Plan a trip to Lisbon");
        assert!(hits[1].message_id.is_some());
        assert!(
            f.chats
                .search(&actor(), "lisbon", 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(f.chats.search(&caller, "lisbon", 1).await.unwrap().len(), 1);
        let error = f.chats.search(&caller, "  ", 10).await.unwrap_err();
        assert_eq!(error.to_string(), "Enter text to search for.");
        let error = f
            .chats
            .search(&caller, &"x".repeat(201), 10)
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Search text must be at most 200 characters."
        );
    }

    #[tokio::test]
    async fn saves_a_standalone_chat_as_a_project() {
        let f = fixture();
        let caller = actor();
        let (model, _) = model(&f, &caller).await;
        let kept = server(&f, &caller).await;
        let input = NewChat {
            model_id: Some(model),
            instructions: "You review contracts.".to_owned(),
            reasoning: Some(ReasoningLevel::High),
            builtin_tools: Some(vec![BuiltinTool::Schedules]),
            tool_overrides: vec![ServerOverride {
                server_id: kept,
                enabled: true,
            }],
            ..NewChat::default()
        };
        let chat = f.chats.create(&caller, input).await.unwrap();
        let file = f
            .files
            .upload(
                &caller,
                FileOwner::Chat(chat.id()),
                "contract.txt",
                Bytes::from_static(b"Terms"),
            )
            .await
            .unwrap();
        f.chats
            .prepare(&caller, chat.id(), send("Review the NDA"))
            .await
            .unwrap();

        let project = f.chats.save_as_project(&caller, chat.id()).await.unwrap();
        assert_eq!(project.name(), "Review the NDA");
        assert_eq!(project.instructions(), "You review contracts.");
        assert_eq!(project.default_model_id(), Some(model));
        assert_eq!(project.reasoning(), Some(ReasoningLevel::High));
        assert_eq!(project.builtin_tools(), [BuiltinTool::Schedules]);
        assert_eq!(project.tool_server_ids(), [kept]);
        assert_eq!(
            f.projects.get(&caller, project.id()).await.unwrap(),
            project
        );

        let moved = f.chats.get(&caller, chat.id()).await.unwrap();
        assert_eq!(moved.project_id(), Some(project.id()));
        assert_eq!(moved.instructions(), "", "nothing is sent twice");
        assert!(moved.tool_overrides().is_empty());
        let files = f.files.list(&caller, project.id()).await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].id(), file.id(), "moved, not copied");
        assert!(files[0].included());
        assert_eq!(
            moved.tool_selection(Some(&project)),
            ToolSelection {
                project: vec![kept],
                overrides: Vec::new(),
            }
        );

        let error = f
            .chats
            .save_as_project(&caller, chat.id())
            .await
            .unwrap_err();
        assert_eq!(error.code(), "CHAT_IN_PROJECT");
        let untitled = f.chats.create(&caller, NewChat::default()).await.unwrap();
        let project = f
            .chats
            .save_as_project(&caller, untitled.id())
            .await
            .unwrap();
        assert_eq!(project.name(), "New project");
        let projects = f
            .projects
            .list(&caller, ProjectQuery::default())
            .await
            .unwrap();
        assert_eq!(projects.len(), 2);
    }

    /// What happens to the chat between `save_as_project`'s read and its write.
    #[derive(Clone, Copy)]
    enum Meanwhile {
        /// Another request saves it as a project first.
        Saved,
        /// Another request deletes it.
        Deleted,
    }

    /// The store's projects, with another request acting on the chat just before each save.
    struct Racing {
        store: Arc<crate::testing::MemoryStore>,
        meanwhile: Meanwhile,
        owner: Actor,
    }

    #[async_trait::async_trait]
    impl ProjectRepository for Racing {
        async fn list(&self, user: UserId, query: ProjectQuery) -> DomainResult<Vec<Project>> {
            ProjectRepository::list(self.store.as_ref(), user, query).await
        }

        async fn get(&self, user: UserId, id: ProjectId) -> DomainResult<Option<Project>> {
            ProjectRepository::get(self.store.as_ref(), user, id).await
        }

        async fn insert(&self, project: &Project) -> DomainResult<()> {
            ProjectRepository::insert(self.store.as_ref(), project).await
        }

        async fn update(&self, project: &Project) -> DomainResult<()> {
            ProjectRepository::update(self.store.as_ref(), project).await
        }

        async fn delete(&self, user: UserId, id: ProjectId) -> DomainResult<bool> {
            ProjectRepository::delete(self.store.as_ref(), user, id).await
        }

        async fn insert_with_chat(&self, project: &Project, chat: ChatId) -> DomainResult<bool> {
            let user = self.owner.user_id;
            match self.meanwhile {
                Meanwhile::Saved => {
                    let first = Project::create(user, "First", project.created_at())?;
                    assert!(self.store.insert_with_chat(&first, chat).await?);
                }
                Meanwhile::Deleted => {
                    assert!(ChatRepository::delete(self.store.as_ref(), user, chat).await?);
                }
            }
            self.store.insert_with_chat(project, chat).await
        }
    }

    #[tokio::test]
    async fn a_chat_saved_or_deleted_meanwhile_is_not_saved_again() {
        for meanwhile in [Meanwhile::Saved, Meanwhile::Deleted] {
            let f = fixture();
            let caller = actor();
            let chat = f.chats.create(&caller, NewChat::default()).await.unwrap();
            let racing = Racing {
                store: f.store.clone(),
                meanwhile,
                owner: caller,
            };
            let chats = ChatService::new(
                ChatRepositories {
                    projects: Arc::new(racing),
                    ..ChatRepositories::shared(f.store.clone())
                },
                f.tools.clone(),
                f.cipher.clone(),
                f.clock.clone(),
                PrivateNetwork::Admins,
            );

            let error = chats.save_as_project(&caller, chat.id()).await.unwrap_err();
            let projects = f
                .projects
                .list(&caller, ProjectQuery::default())
                .await
                .unwrap();
            match meanwhile {
                Meanwhile::Saved => {
                    assert_eq!(error.code(), "CHAT_IN_PROJECT");
                    let names: Vec<&str> = projects.iter().map(Project::name).collect();
                    assert_eq!(names, ["First"], "only the first save made a project");
                }
                Meanwhile::Deleted => {
                    assert!(is_not_found(&error, "Chat"), "{error:?}");
                    assert!(projects.is_empty());
                }
            }
        }
    }

    #[tokio::test]
    async fn saves_chat_instructions_to_the_project() {
        let f = fixture();
        let caller = actor();
        let project = project(&f, &caller, "Work").await;
        let chat = f
            .chats
            .create(
                &caller,
                NewChat {
                    project_id: Some(project.id()),
                    instructions: "Cite sources.".to_owned(),
                    ..NewChat::default()
                },
            )
            .await
            .unwrap();
        let saved = f.chats.save_instructions(&caller, chat.id()).await.unwrap();
        assert_eq!(saved.instructions(), "Cite sources.");
        let chat = f
            .chats
            .update(
                &caller,
                chat.id(),
                ChatUpdate {
                    instructions: Some("Answer in French.".to_owned()),
                    ..ChatUpdate::default()
                },
            )
            .await
            .unwrap();
        let saved = f.chats.save_instructions(&caller, chat.id()).await.unwrap();
        assert_eq!(saved.instructions(), "Cite sources.\n\nAnswer in French.");
        assert_eq!(
            f.chats
                .get(&caller, chat.id())
                .await
                .unwrap()
                .instructions(),
            ""
        );
        let error = f
            .chats
            .save_instructions(&caller, chat.id())
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "This chat has no instructions to save.");

        let standalone = f
            .chats
            .create(
                &caller,
                NewChat {
                    instructions: "x".to_owned(),
                    ..NewChat::default()
                },
            )
            .await
            .unwrap();
        let error = f
            .chats
            .save_instructions(&caller, standalone.id())
            .await
            .unwrap_err();
        assert_eq!(error.code(), "CHAT_NOT_IN_PROJECT");
    }

    #[tokio::test]
    async fn recovers_streaming_messages_only() {
        let f = fixture();
        let caller = actor();
        let chat = chat(&f, &caller).await;
        let paused = pause(&f, &caller, &chat).await;
        let streaming = Message::assistant(
            caller.user_id,
            chat.id(),
            chat.model_id().unwrap(),
            ProviderId::generate(),
            f.clock.now(),
        );
        let streaming = Message::restore(MessageParts {
            provider_id: None,
            parts: vec![Part::Text {
                text: "Half an ans".to_owned(),
            }],
            ..streaming.to_parts()
        });
        f.store.append(caller.user_id, &streaming).await.unwrap();
        assert_eq!(f.chats.recover().await.unwrap(), 1);
        let history = f.chats.messages(&caller, chat.id()).await.unwrap();
        assert_eq!(history[0].id(), paused.id());
        assert_eq!(history[0].status(), MessageStatus::AwaitingApproval);
        assert_eq!(history[1].status(), MessageStatus::Error);
        assert_eq!(history[1].error(), Some(INTERRUPTED));
        assert_eq!(
            history[1].text_content(),
            "Half an ans",
            "partial text stays"
        );
        assert_eq!(f.chats.recover().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn deletes_follow_the_database_cascades() {
        let f = fixture();
        let caller = actor();
        let (model, provider) = model(&f, &caller).await;
        let project = project(&f, &caller, "Work").await;
        let server = server(&f, &caller).await;
        let in_project = f
            .chats
            .create(
                &caller,
                NewChat {
                    project_id: Some(project.id()),
                    model_id: Some(model),
                    tool_overrides: vec![ServerOverride {
                        server_id: server,
                        enabled: true,
                    }],
                    ..NewChat::default()
                },
            )
            .await
            .unwrap();
        let standalone = f
            .chats
            .create(
                &caller,
                NewChat {
                    model_id: Some(model),
                    tool_overrides: vec![ServerOverride {
                        server_id: server,
                        enabled: true,
                    }],
                    ..NewChat::default()
                },
            )
            .await
            .unwrap();
        f.chats
            .prepare(&caller, standalone.id(), send("Hi"))
            .await
            .unwrap();

        f.tools.delete(&caller, server).await.unwrap();
        let chat = f.chats.get(&caller, standalone.id()).await.unwrap();
        assert!(
            chat.tool_overrides().is_empty(),
            "chat_tool_servers cascade"
        );

        let details = f.providers.get(&caller, provider).await.unwrap();
        assert_eq!(details.usage.chats, 2);
        f.providers.delete(&caller, provider).await.unwrap();
        let chat = f.chats.get(&caller, standalone.id()).await.unwrap();
        assert_eq!(chat.model_id(), None, "ON DELETE SET NULL");
        let error = f
            .chats
            .prepare(&caller, standalone.id(), send("Again"))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "Choose a model for this chat.");

        f.projects.delete(&caller, project.id()).await.unwrap();
        assert!(f.chats.get(&caller, in_project.id()).await.is_err());
        assert!(f.chats.get(&caller, standalone.id()).await.is_ok());
    }
}
