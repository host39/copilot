//! Turn preparation: validate, decrypt, save the user message or tail change, and hand the engine
//! a `PreparedTurn` (`docs/design/copilot.md`, Turn Lifecycle, step 2).
//!
//! Every check runs before the first write, so a failed `prepare` changes nothing.

use std::collections::HashSet;

use futures_util::future::{try_join, try_join3, try_join4};

use crate::domains::chat::context::TurnContext;
use crate::domains::chat::entities::Chat;
use crate::domains::chat::message::{
    CallDecision, Decision, DecisionScope, Message, MessageStatus, Part, Role, SKIPPED,
    skipped_results,
};
use crate::domains::chat::service::ChatService;
use crate::domains::chat::turn::{PreparedTurn, RunMode, TurnInput};
use crate::domains::file::{FileKind, FileOwner, MediaType, StoredFile};
use crate::domains::project::Project;
use crate::domains::provider::{Capabilities, Model, Provider};
use crate::domains::schedule::{Firing, Schedule};
use crate::domains::settings::UserSettings;
use crate::domains::tool::EffectiveTools;
use crate::ports::llm_gateway::ModelTarget;
use crate::shared::actor::Actor;
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{ChatId, FileId, MessageId, ModelId, ProjectId};

/// Code of a model that cannot answer (turned off, no longer listed, or its connection is off).
const MODEL_UNAVAILABLE: &str = "MODEL_UNAVAILABLE";
/// Code of a decision on a message that does not wait for one.
const NOT_AWAITING_APPROVAL: &str = "NOT_AWAITING_APPROVAL";
/// Longest reason for a denial, in characters.
const REASON_MAX_CHARS: usize = 500;

/// Everything a turn reads before it writes.
struct Loaded {
    settings: UserSettings,
    project: Option<Project>,
    schedule: Option<Schedule>,
    model: Model,
    provider: Provider,
    capabilities: Capabilities,
    target: ModelTarget,
    files: Vec<StoredFile>,
    history: Vec<Message>,
}

impl ChatService {
    /// Validates a turn of `chat` and saves its user message (or tail change). Loads the chat,
    /// settings, project, schedule, model, provider, project files, and history (at most 4 reads
    /// at once); requires a model that is enabled, available, and on an enabled connection;
    /// opens the key. Then, by input:
    ///
    /// - `Send`: attachments must be this chat's files. Without `edit_from`, a paused round is
    ///   closed first (pending calls denied with "Skipped by user.", a `tool` message written);
    ///   with it, the edited user message and everything after it are replaced.
    /// - `Regenerate`: deletes every message after the last user message.
    /// - `Decide`: the decisions must cover exactly the pending calls of the chat's latest
    ///   assistant message while it awaits approval; "Approve for this chat" adds the alias to
    ///   the chat. The turn continues from the paused message's snapshot.
    pub async fn prepare(
        &self,
        actor: &Actor,
        id: ChatId,
        input: TurnInput,
    ) -> DomainResult<PreparedTurn> {
        let chat = self.get(actor, id).await?;
        let loaded = self.load(actor, &chat).await?;
        match input {
            TurnInput::Send {
                text,
                file_ids,
                edit_from,
            } => {
                self.send(actor, chat, loaded, &text, &file_ids, edit_from)
                    .await
            }
            TurnInput::Regenerate => self.regenerate(actor, chat, loaded).await,
            TurnInput::Decide {
                message_id,
                decisions,
            } => {
                self.decide(actor, chat, loaded, message_id, decisions)
                    .await
            }
        }
    }

    /// Starts the unattended run of a claimed schedule (or a Run now): a new chat titled
    /// `<schedule name> · <local date>` with the schedule's project, model, and reasoning, and
    /// the prompt as its user message. A standalone schedule runs without tools.
    pub async fn prepare_scheduled(&self, firing: Firing) -> DomainResult<PreparedTurn> {
        let Firing {
            schedule,
            actor,
            at,
        } = firing;
        let user = actor.user_id;
        let (settings, project, model) = try_join3(
            self.stores.settings.get(user),
            self.project(&actor, schedule.project_id()),
            self.model(&actor, schedule.model_id()),
        )
        .await?;
        if let (Some(id), None) = (schedule.project_id(), &project) {
            return Err(DomainError::not_found("Project", id.to_string()));
        }
        let Some(model) = model else {
            return Err(
                DomainError::validation("Choose a model for this schedule.").with_field("modelId")
            );
        };
        let provider = self.provider(&actor, &model).await?;
        let (capabilities, target) = self.target(&actor, &model, &provider)?;
        let files = self.project_files(&actor, project.as_ref()).await?;

        let title = format!("{} · {}", schedule.name(), schedule.rule().local_date(at));
        let chat = Chat::scheduled(user, schedule.id(), title, self.clock.now())
            .with_project(schedule.project_id())
            .with_model(schedule.model_id())
            .with_reasoning(schedule.reasoning());
        self.stores.chats.insert(&chat).await?;
        let prompt = Message::user(
            user,
            chat.id(),
            vec![Part::Text {
                text: schedule.prompt().to_owned(),
            }],
            self.clock.now(),
        );
        self.stores.messages.append(user, &prompt).await?;
        let tools = if project.is_some() && capabilities.tools {
            self.tools
                .effective_tools(&actor, &chat.tool_selection(project.as_ref()))
                .await?
        } else {
            EffectiveTools::default()
        };
        let settings = settings.unwrap_or_else(|| UserSettings::empty(user, self.clock.now()));
        Ok(PreparedTurn {
            actor,
            mode: RunMode::Unattended {
                on_ask: schedule.on_ask(),
            },
            settings,
            project,
            schedule: Some(schedule),
            model,
            provider,
            capabilities,
            target,
            files,
            tools,
            history: vec![prompt],
            snapshot: None,
            chat,
        })
    }

    async fn send(
        &self,
        actor: &Actor,
        chat: Chat,
        loaded: Loaded,
        text: &str,
        file_ids: &[FileId],
        edit_from: Option<MessageId>,
    ) -> DomainResult<PreparedTurn> {
        let user = actor.user_id;
        let text = text.trim();
        let mut parts = Vec::new();
        if !text.is_empty() {
            parts.push(Part::Text {
                text: text.to_owned(),
            });
        }
        let mut seen = HashSet::new();
        for id in file_ids.iter().filter(|id| seen.insert(**id)) {
            parts.push(self.attachment(actor, chat.id(), *id).await?);
        }
        if parts.is_empty() {
            return Err(
                DomainError::validation("Type a message or attach a file.").with_field("text")
            );
        }
        if let Some(from) = edit_from {
            let edited = loaded
                .history
                .iter()
                .any(|message| message.id() == from && message.role() == Role::User);
            if !edited {
                return Err(DomainError::not_found("Message", from.to_string()));
            }
        }

        let now = self.clock.now();
        let message = Message::user(user, chat.id(), parts, now);
        match edit_from {
            Some(from) => {
                self.stores
                    .messages
                    .replace_tail(user, chat.id(), from, &message)
                    .await?;
            }
            None => {
                if let Some(paused) = paused(&loaded.history) {
                    self.skip(actor, paused).await?;
                }
                self.stores.messages.append(user, &message).await?;
            }
        }
        let title_source = if text.is_empty() {
            message.parts().iter().find_map(|part| match part {
                Part::File { name, .. } => Some(name.as_str()),
                _ => None,
            })
        } else {
            Some(text)
        };
        let mut chat = chat;
        if chat.title().is_empty()
            && let Some(source) = title_source
        {
            chat = chat.titled_by(source);
            self.stores.chats.update(&chat).await?;
        }
        self.finish(actor, chat, loaded, None).await
    }

    async fn regenerate(
        &self,
        actor: &Actor,
        chat: Chat,
        loaded: Loaded,
    ) -> DomainResult<PreparedTurn> {
        let Some(last) = loaded
            .history
            .iter()
            .rev()
            .find(|message| message.role() == Role::User)
        else {
            return Err(DomainError::validation(
                "There is no message to answer yet.",
            ));
        };
        self.stores
            .messages
            .delete_after(actor.user_id, chat.id(), last.id())
            .await?;
        self.finish(actor, chat, loaded, None).await
    }

    async fn decide(
        &self,
        actor: &Actor,
        chat: Chat,
        loaded: Loaded,
        message_id: MessageId,
        decisions: Vec<CallDecision>,
    ) -> DomainResult<PreparedTurn> {
        let user = actor.user_id;
        let Some(message) = loaded
            .history
            .iter()
            .find(|message| message.id() == message_id)
        else {
            return Err(DomainError::not_found("Message", message_id.to_string()));
        };
        if paused(&loaded.history).map(Message::id) != Some(message_id) {
            return Err(not_awaiting());
        }
        let decisions = checked(message, decisions)?;
        let hash = message
            .context_hash()
            .ok_or_else(|| missing_snapshot(message_id))?;
        let snapshot = self
            .stores
            .contexts
            .get(user, chat.id(), hash)
            .await?
            .ok_or_else(|| missing_snapshot(message_id))?;
        let approved: Vec<_> = message
            .pending_calls()
            .filter(|call| {
                decisions.iter().any(|decision| {
                    decision.call_id == call.id
                        && decision.decision == Decision::Approve
                        && decision.scope == DecisionScope::Chat
                })
            })
            .map(|call| call.alias.clone())
            .collect();

        if !self
            .stores
            .messages
            .decide(user, message_id, &decisions)
            .await?
        {
            return Err(not_awaiting());
        }
        let mut chat = chat;
        if !approved.is_empty() {
            for alias in approved {
                chat = chat.approving(alias);
            }
            self.stores.chats.update(&chat).await?;
        }
        self.finish(actor, chat, loaded, Some(snapshot)).await
    }

    /// Closes a paused round the user skipped: pending calls denied, and a `tool` message with a
    /// result for every call. A concurrent decision wins; then nothing is written.
    async fn skip(&self, actor: &Actor, paused: &Message) -> DomainResult<()> {
        let decisions: Vec<CallDecision> = paused
            .pending_calls()
            .map(|call| CallDecision {
                call_id: call.id,
                decision: Decision::Deny,
                scope: DecisionScope::Once,
                reason: Some(SKIPPED.to_owned()),
            })
            .collect();
        let user = actor.user_id;
        if self
            .stores
            .messages
            .decide(user, paused.id(), &decisions)
            .await?
        {
            let results = Message::tool(
                user,
                paused.chat_id(),
                skipped_results(paused),
                self.clock.now(),
            );
            self.stores.messages.append(user, &results).await?;
        }
        Ok(())
    }

    /// The prepared turn after the writes: the tools (none for a decision, which keeps its
    /// snapshot's) and the history read again.
    async fn finish(
        &self,
        actor: &Actor,
        chat: Chat,
        loaded: Loaded,
        snapshot: Option<TurnContext>,
    ) -> DomainResult<PreparedTurn> {
        let tools = if snapshot.is_none() && loaded.capabilities.tools {
            self.tools
                .effective_tools(actor, &chat.tool_selection(loaded.project.as_ref()))
                .await?
        } else {
            EffectiveTools::default()
        };
        let history = self
            .stores
            .messages
            .history(actor.user_id, chat.id())
            .await?;
        Ok(PreparedTurn {
            actor: *actor,
            chat,
            mode: RunMode::Interactive,
            settings: loaded.settings,
            project: loaded.project,
            schedule: loaded.schedule,
            model: loaded.model,
            provider: loaded.provider,
            capabilities: loaded.capabilities,
            target: loaded.target,
            files: loaded.files,
            tools,
            history,
            snapshot,
        })
    }

    async fn load(&self, actor: &Actor, chat: &Chat) -> DomainResult<Loaded> {
        let user = actor.user_id;
        let schedule = async {
            match chat.schedule_id() {
                Some(id) => self.stores.schedules.get(user, id).await,
                None => Ok(None),
            }
        };
        let (settings, project, schedule, history) = try_join4(
            self.stores.settings.get(user),
            self.project(actor, chat.project_id()),
            schedule,
            self.stores.messages.history(user, chat.id()),
        )
        .await?;
        let (model, files) = try_join(
            self.model(actor, chat.model_id()),
            self.project_files(actor, project.as_ref()),
        )
        .await?;
        let Some(model) = model else {
            return Err(
                DomainError::validation("Choose a model for this chat.").with_field("modelId")
            );
        };
        let provider = self.provider(actor, &model).await?;
        let (capabilities, target) = self.target(actor, &model, &provider)?;
        Ok(Loaded {
            settings: settings.unwrap_or_else(|| UserSettings::empty(user, self.clock.now())),
            project,
            schedule,
            model,
            provider,
            capabilities,
            target,
            files,
            history,
        })
    }

    async fn project(&self, actor: &Actor, id: Option<ProjectId>) -> DomainResult<Option<Project>> {
        match id {
            Some(id) => self.stores.projects.get(actor.user_id, id).await,
            None => Ok(None),
        }
    }

    async fn model(&self, actor: &Actor, id: Option<ModelId>) -> DomainResult<Option<Model>> {
        match id {
            Some(id) => self.stores.models.get(actor.user_id, id).await,
            None => Ok(None),
        }
    }

    /// The model's connection; the model must be enabled and available and the connection
    /// enabled.
    async fn provider(&self, actor: &Actor, model: &Model) -> DomainResult<Provider> {
        if !model.enabled() {
            return Err(DomainError::business_rule(
                "This model is turned off. Turn it on in the provider settings or choose another model.",
                MODEL_UNAVAILABLE,
            ));
        }
        if !model.available() {
            return Err(DomainError::business_rule(
                "This model is no longer offered by its provider. Choose another model.",
                MODEL_UNAVAILABLE,
            ));
        }
        let provider = self
            .stores
            .providers
            .get(actor.user_id, model.provider_id())
            .await?
            .ok_or_else(|| DomainError::not_found("Provider", model.provider_id().to_string()))?;
        if !provider.enabled() {
            return Err(DomainError::business_rule(
                "This model's provider connection is turned off.",
                MODEL_UNAVAILABLE,
            ));
        }
        Ok(provider)
    }

    /// What the model can do and the adapter's target with the opened key (`SECRET_UNREADABLE`
    /// when the key no longer opens).
    fn target(
        &self,
        actor: &Actor,
        model: &Model,
        provider: &Provider,
    ) -> DomainResult<(Capabilities, ModelTarget)> {
        let api_key = provider
            .api_key_sealed()
            .map(|sealed| self.cipher.open(sealed, &provider.key_context()))
            .transpose()?;
        let target = model.target(
            provider.target(api_key, self.network.scope(actor)),
            provider.dialect(),
        );
        Ok((model.effective(provider), target))
    }

    /// The project's included files, oldest first.
    async fn project_files(
        &self,
        actor: &Actor,
        project: Option<&Project>,
    ) -> DomainResult<Vec<StoredFile>> {
        let Some(project) = project else {
            return Ok(Vec::new());
        };
        let mut files = self
            .stores
            .files
            .list(actor.user_id, FileOwner::Project(project.id()))
            .await?;
        files.retain(StoredFile::included);
        Ok(files)
    }

    /// An attachment part for one of this chat's files.
    async fn attachment(&self, actor: &Actor, chat: ChatId, id: FileId) -> DomainResult<Part> {
        let file = self
            .stores
            .files
            .get(actor.user_id, id)
            .await?
            .filter(|file| file.owner() == FileOwner::Chat(chat))
            .ok_or_else(|| DomainError::not_found("File", id.to_string()))?;
        Ok(match file.media() {
            Some(media) if media.kind() == FileKind::Audio => Part::Audio {
                file_id: file.id(),
                format: audio_format(media).to_owned(),
                transcript: None,
                duration_ms: None,
            },
            _ => Part::File {
                file_id: file.id(),
                name: file.name().to_owned(),
                media_type: file.media_type().to_owned(),
            },
        })
    }
}

/// The `format` of an audio part (`input_audio` names).
fn audio_format(media: MediaType) -> &'static str {
    match media {
        MediaType::Mpeg => "mp3",
        _ => "wav",
    }
}

/// The chat's latest assistant message when it waits for decisions.
fn paused(history: &[Message]) -> Option<&Message> {
    history
        .iter()
        .rev()
        .find(|message| message.role() == Role::Assistant)
        .filter(|message| message.status() == MessageStatus::AwaitingApproval)
}

/// The decisions with trimmed reasons, when they cover each pending call of `message` exactly
/// once.
fn checked(message: &Message, decisions: Vec<CallDecision>) -> DomainResult<Vec<CallDecision>> {
    let pending: HashSet<_> = message.pending_calls().map(|call| call.id).collect();
    let mut decided = HashSet::new();
    let exact = decisions
        .iter()
        .all(|decision| pending.contains(&decision.call_id) && decided.insert(decision.call_id))
        && decided.len() == pending.len();
    if !exact {
        return Err(DomainError::validation(
            "Decide each pending tool call of this message exactly once.",
        )
        .with_field("decisions"));
    }
    decisions
        .into_iter()
        .map(|mut decision| {
            decision.reason = decision
                .reason
                .map(|reason| reason.trim().to_owned())
                .filter(|reason| !reason.is_empty() && decision.decision == Decision::Deny);
            if decision
                .reason
                .as_ref()
                .is_some_and(|reason| reason.chars().count() > REASON_MAX_CHARS)
            {
                return Err(DomainError::validation(format!(
                    "A reason must be at most {REASON_MAX_CHARS} characters."
                ))
                .with_field("reason"));
            }
            Ok(decision)
        })
        .collect()
}

fn not_awaiting() -> DomainError {
    DomainError::business_rule(
        "This message is not waiting for a decision.",
        NOT_AWAITING_APPROVAL,
    )
}

fn missing_snapshot(message: MessageId) -> DomainError {
    DomainError::Internal {
        key: "core.chat.decide.snapshot-missing".to_owned(),
        source: Some(format!("message {message}").into()),
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use chrono::Duration;

    use super::*;
    use crate::domains::chat::message::{Approval, ToolCallPart, ToolResultPart, ToolStatus};
    use crate::domains::chat::ports::{ChatQuery, MessageRepository};
    use crate::domains::chat::service::NewChat;
    use crate::domains::chat::service::fixture::{
        Fixture, chat, fixture, model, pause, send, server,
    };
    use crate::domains::file::FileUpdate;
    use crate::domains::project::NewProject;
    use crate::domains::provider::{ModelUpdate, ProviderUpdate};
    use crate::domains::schedule::{NewSchedule, OnAsk};
    use crate::domains::settings::SettingsRepository;
    use crate::ports::clock::Clock;
    use crate::ports::secret_cipher::SECRET_UNREADABLE;
    use crate::shared::reasoning::ReasoningLevel;
    use crate::testing::{actor, remote_model};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n....";
    const WAV: &[u8] = b"RIFF\x24\x00\x00\x00WAVEfmt ";

    fn is_not_found(error: &DomainError, entity: &str) -> bool {
        matches!(error, DomainError::EntityNotFound { entity_name, .. } if entity_name == entity)
    }

    async fn history(f: &Fixture, caller: &Actor, chat: &Chat) -> Vec<Message> {
        f.store.history(caller.user_id, chat.id()).await.unwrap()
    }

    fn texts(history: &[Message]) -> Vec<(Role, String)> {
        history
            .iter()
            .map(|message| (message.role(), message.text_content()))
            .collect()
    }

    fn decide(message: &Message, decision: Decision, scope: DecisionScope) -> TurnInput {
        TurnInput::Decide {
            message_id: message.id(),
            decisions: message
                .pending_calls()
                .map(|call| CallDecision {
                    call_id: call.id,
                    decision,
                    scope,
                    reason: None,
                })
                .collect(),
        }
    }

    #[tokio::test]
    async fn send_saves_the_message_and_prepares_the_turn() {
        let f = fixture();
        let caller = actor();
        let (model, _) = model(&f, &caller).await;
        let server = server(&f, &caller).await;
        let input = NewProject {
            name: "Work".to_owned(),
            default_model_id: Some(model),
            tool_server_ids: vec![server],
            ..NewProject::default()
        };
        let project = f.projects.create(&caller, input).await.unwrap();
        let spec = Bytes::from_static(b"# Spec");
        let included = f
            .files
            .upload(&caller, FileOwner::Project(project.id()), "spec.md", spec)
            .await
            .unwrap();
        let old = Bytes::from_static(b"# Old");
        let left_out = f
            .files
            .upload(&caller, FileOwner::Project(project.id()), "old.md", old)
            .await
            .unwrap();
        let exclude = FileUpdate {
            included: Some(false),
            ..FileUpdate::default()
        };
        f.files
            .update(&caller, left_out.id(), exclude)
            .await
            .unwrap();
        let draft = NewChat {
            project_id: Some(project.id()),
            model_id: Some(model),
            reasoning: Some(ReasoningLevel::Low),
            ..NewChat::default()
        };
        let chat = f.chats.create(&caller, draft).await.unwrap();
        let png = Bytes::from_static(PNG);
        let image = f
            .files
            .upload(&caller, FileOwner::Chat(chat.id()), "shot.png", png)
            .await
            .unwrap();
        let wav = Bytes::from_static(WAV);
        let clip = f
            .files
            .upload(&caller, FileOwner::Chat(chat.id()), "clip.wav", wav)
            .await
            .unwrap();
        let settings = UserSettings::empty(caller.user_id, f.clock.now())
            .with_instructions("Call me Sam.", f.clock.now());
        f.store.save(&settings).await.unwrap();

        let input = TurnInput::Send {
            text: "  What changed\nin the spec?  ".to_owned(),
            file_ids: vec![image.id(), clip.id(), image.id()],
            edit_from: None,
        };
        let turn = f.chats.prepare(&caller, chat.id(), input).await.unwrap();
        assert_eq!(turn.actor, caller);
        assert_eq!(turn.mode, RunMode::Interactive);
        assert_eq!(turn.chat.title(), "What changed in the spec?");
        let stored = f.chats.get(&caller, chat.id()).await.unwrap();
        assert_eq!(stored.title(), turn.chat.title());
        assert_eq!(turn.settings.instructions(), "Call me Sam.");
        assert_eq!(turn.project.as_ref().map(Project::id), Some(project.id()));
        assert_eq!(turn.model.id(), model);
        assert!(turn.capabilities.tools);
        let key = turn
            .target
            .provider
            .api_key
            .as_ref()
            .expect("the opened key");
        assert_eq!(key.expose(), b"sk-1");
        assert_eq!(turn.target.remote_id, "gpt-5.6-terra");
        let files: Vec<FileId> = turn.files.iter().map(StoredFile::id).collect();
        assert_eq!(files, [included.id()], "included project files only");
        let aliases: Vec<&str> = turn
            .tools
            .tools
            .iter()
            .map(|tool| tool.alias.as_str())
            .collect();
        assert_eq!(aliases, ["github__search"]);
        assert!(turn.snapshot.is_none());

        let message = turn.last_message().expect("the user message");
        assert_eq!(message.role(), Role::User);
        assert_eq!(
            message.parts(),
            [
                Part::Text {
                    text: "What changed\nin the spec?".to_owned()
                },
                Part::File {
                    file_id: image.id(),
                    name: "shot.png".to_owned(),
                    media_type: "image/png".to_owned(),
                },
                Part::Audio {
                    file_id: clip.id(),
                    format: "wav".to_owned(),
                    transcript: None,
                    duration_ms: None,
                },
            ],
            "trimmed text, each attachment once, audio as an audio part"
        );
        assert_eq!(turn.history, history(&f, &caller, &chat).await);

        let turn = f
            .chats
            .prepare(&caller, chat.id(), send("And now?"))
            .await
            .unwrap();
        assert_eq!(
            turn.chat.title(),
            "What changed in the spec?",
            "titled once"
        );
        assert_eq!(turn.history.len(), 2);
    }

    #[tokio::test]
    async fn an_attachment_only_message_is_titled_by_the_file_name() {
        let f = fixture();
        let caller = actor();
        let chat = chat(&f, &caller).await;
        let png = Bytes::from_static(PNG);
        let image = f
            .files
            .upload(&caller, FileOwner::Chat(chat.id()), "receipt.png", png)
            .await
            .unwrap();
        let input = TurnInput::Send {
            text: " ".to_owned(),
            file_ids: vec![image.id()],
            edit_from: None,
        };
        let turn = f.chats.prepare(&caller, chat.id(), input).await.unwrap();
        assert_eq!(turn.chat.title(), "receipt.png");
    }

    #[tokio::test]
    async fn validates_before_writing_anything() {
        let f = fixture();
        let caller = actor();
        let other = chat(&f, &caller).await;
        let chat = chat(&f, &caller).await;
        let png = Bytes::from_static(PNG);
        let foreign = f
            .files
            .upload(&caller, FileOwner::Chat(other.id()), "a.png", png)
            .await
            .unwrap();

        let error = f
            .chats
            .prepare(&caller, chat.id(), send("  \n "))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "Type a message or attach a file.");
        let input = TurnInput::Send {
            text: "Look".to_owned(),
            file_ids: vec![foreign.id()],
            edit_from: None,
        };
        let error = f
            .chats
            .prepare(&caller, chat.id(), input)
            .await
            .unwrap_err();
        assert!(is_not_found(&error, "File"), "another chat's file");
        let input = TurnInput::Send {
            text: "Edit".to_owned(),
            file_ids: Vec::new(),
            edit_from: Some(MessageId::generate()),
        };
        let error = f
            .chats
            .prepare(&caller, chat.id(), input)
            .await
            .unwrap_err();
        assert!(is_not_found(&error, "Message"));
        let error = f
            .chats
            .prepare(&caller, chat.id(), TurnInput::Regenerate)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "There is no message to answer yet.");
        assert!(history(&f, &caller, &chat).await.is_empty());
        assert_eq!(f.chats.get(&caller, chat.id()).await.unwrap().title(), "");
    }

    #[tokio::test]
    async fn requires_a_usable_model_and_a_readable_key() {
        let f = fixture();
        let caller = actor();
        let chat = chat(&f, &caller).await;
        let model = chat.model_id().unwrap();
        let provider = f
            .chats
            .prepare(&caller, chat.id(), send("Hi"))
            .await
            .unwrap();
        let provider = provider.provider.id();

        let off = ModelUpdate {
            enabled: Some(false),
            ..ModelUpdate::default()
        };
        f.models.update(&caller, model, off).await.unwrap();
        let error = f
            .chats
            .prepare(&caller, chat.id(), send("Hi"))
            .await
            .unwrap_err();
        assert_eq!(error.code(), MODEL_UNAVAILABLE);
        assert_eq!(
            error.to_string(),
            "This model is turned off. Turn it on in the provider settings or choose another model."
        );
        let on = ModelUpdate {
            enabled: Some(true),
            ..ModelUpdate::default()
        };
        f.models.update(&caller, model, on).await.unwrap();

        f.gateway.push_models(Vec::new());
        f.providers.refresh(&caller, provider).await.unwrap();
        let error = f
            .chats
            .prepare(&caller, chat.id(), send("Hi"))
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "This model is no longer offered by its provider. Choose another model."
        );
        f.gateway.push_models(vec![remote_model("gpt-5.6-terra")]);
        f.providers.refresh(&caller, provider).await.unwrap();

        let disabled = ProviderUpdate {
            enabled: Some(false),
            ..ProviderUpdate::default()
        };
        f.providers
            .update(&caller, provider, disabled)
            .await
            .unwrap();
        let error = f
            .chats
            .prepare(&caller, chat.id(), send("Hi"))
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "This model's provider connection is turned off."
        );
        let enabled = ProviderUpdate {
            enabled: Some(true),
            ..ProviderUpdate::default()
        };
        f.providers
            .update(&caller, provider, enabled)
            .await
            .unwrap();

        f.cipher.rotate();
        let error = f
            .chats
            .prepare(&caller, chat.id(), send("Hi"))
            .await
            .unwrap_err();
        assert_eq!(error.code(), SECRET_UNREADABLE);
        assert_eq!(
            history(&f, &caller, &chat).await.len(),
            1,
            "nothing saved on failure"
        );

        let no_model = f.chats.create(&caller, NewChat::default()).await.unwrap();
        let error = f
            .chats
            .prepare(&caller, no_model.id(), send("Hi"))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "Choose a model for this chat.");
    }

    #[tokio::test]
    async fn a_new_message_skips_pending_calls() {
        let f = fixture();
        let caller = actor();
        let chat = chat(&f, &caller).await;
        f.chats
            .prepare(&caller, chat.id(), send("Find it"))
            .await
            .unwrap();
        let paused = pause(&f, &caller, &chat).await;

        let turn = f
            .chats
            .prepare(&caller, chat.id(), send("Never mind"))
            .await
            .unwrap();
        let roles: Vec<Role> = turn.history.iter().map(Message::role).collect();
        assert_eq!(roles, [Role::User, Role::Assistant, Role::Tool, Role::User]);
        let assistant = &turn.history[1];
        assert_eq!(assistant.status(), MessageStatus::Complete);
        let approvals: Vec<(Approval, Option<&str>)> = assistant
            .calls()
            .map(|call| (call.approval, call.deny_reason.as_deref()))
            .collect();
        assert_eq!(
            approvals,
            [
                (Approval::Denied, Some(SKIPPED)),
                (Approval::Denied, Some(SKIPPED)),
                (Approval::Auto, None),
            ]
        );
        let ids: Vec<_> = paused.calls().map(|call| call.id).collect();
        assert_eq!(
            turn.history[2].parts(),
            [
                Part::ToolResult(ToolResultPart::skipped(ids[0], ToolStatus::Denied, SKIPPED)),
                Part::ToolResult(ToolResultPart::skipped(ids[1], ToolStatus::Denied, SKIPPED)),
                Part::ToolResult(ToolResultPart::skipped(
                    ids[2],
                    ToolStatus::Cancelled,
                    SKIPPED
                )),
            ],
            "every call has a result before the next request"
        );
        assert_eq!(turn.history[3].text_content(), "Never mind");
    }

    #[tokio::test]
    async fn edit_replaces_the_tail_and_regenerate_drops_the_answer() {
        let f = fixture();
        let caller = actor();
        let chat = chat(&f, &caller).await;
        f.chats
            .prepare(&caller, chat.id(), send("First"))
            .await
            .unwrap();
        let second = f
            .chats
            .prepare(&caller, chat.id(), send("Second"))
            .await
            .unwrap();
        let second = second.last_message().unwrap().id();
        pause(&f, &caller, &chat).await;

        let error = f
            .chats
            .prepare(
                &caller,
                chat.id(),
                TurnInput::Send {
                    text: "x".to_owned(),
                    file_ids: Vec::new(),
                    edit_from: Some(history(&f, &caller, &chat).await[2].id()),
                },
            )
            .await
            .unwrap_err();
        assert!(
            is_not_found(&error, "Message"),
            "only user messages can be edited"
        );

        let edit = TurnInput::Send {
            text: "Second, edited".to_owned(),
            file_ids: Vec::new(),
            edit_from: Some(second),
        };
        let turn = f.chats.prepare(&caller, chat.id(), edit).await.unwrap();
        assert_eq!(
            texts(&turn.history),
            [
                (Role::User, "First".to_owned()),
                (Role::User, "Second, edited".to_owned()),
            ]
        );

        pause(&f, &caller, &chat).await;
        let turn = f
            .chats
            .prepare(&caller, chat.id(), TurnInput::Regenerate)
            .await
            .unwrap();
        assert_eq!(
            turn.history.len(),
            2,
            "the answer after the last user message is gone"
        );
        assert_eq!(
            turn.last_message().unwrap().text_content(),
            "Second, edited"
        );
    }

    #[tokio::test]
    async fn decisions_continue_from_the_snapshot() {
        let f = fixture();
        let caller = actor();
        let chat = chat(&f, &caller).await;
        f.chats
            .prepare(&caller, chat.id(), send("Clean up"))
            .await
            .unwrap();
        let paused = pause(&f, &caller, &chat).await;
        let calls: Vec<_> = paused.pending_calls().cloned().collect();
        let input = TurnInput::Decide {
            message_id: paused.id(),
            decisions: vec![
                CallDecision {
                    call_id: calls[0].id,
                    decision: Decision::Approve,
                    scope: DecisionScope::Chat,
                    reason: Some("ignored for approvals".to_owned()),
                },
                CallDecision {
                    call_id: calls[1].id,
                    decision: Decision::Deny,
                    scope: DecisionScope::Once,
                    reason: Some("  Not that one.  ".to_owned()),
                },
            ],
        };
        let turn = f.chats.prepare(&caller, chat.id(), input).await.unwrap();
        assert_eq!(turn.mode, RunMode::Interactive);
        let snapshot = turn.snapshot.as_ref().expect("the paused turn's snapshot");
        assert_eq!(snapshot.system, "Be brief.");
        assert!(
            turn.tools.tools.is_empty(),
            "the snapshot's tools, not today's"
        );
        assert_eq!(turn.chat.approved_tools(), [calls[0].alias.clone()]);
        let stored = f.chats.get(&caller, chat.id()).await.unwrap();
        assert_eq!(stored.approved_tools(), [calls[0].alias.clone()]);

        let decided = turn.last_message().unwrap();
        assert_eq!(decided.id(), paused.id());
        assert_eq!(decided.status(), MessageStatus::Complete);
        let approvals: Vec<(Approval, Option<&str>)> = decided
            .calls()
            .map(|call| (call.approval, call.deny_reason.as_deref()))
            .collect();
        assert_eq!(
            approvals,
            [
                (Approval::Approved, None),
                (Approval::Denied, Some("Not that one.")),
                (Approval::Auto, None),
            ]
        );

        let again = decide(&paused, Decision::Approve, DecisionScope::Once);
        let error = f
            .chats
            .prepare(&caller, chat.id(), again)
            .await
            .unwrap_err();
        assert_eq!(
            error.code(),
            NOT_AWAITING_APPROVAL,
            "each decision applies once"
        );
    }

    #[tokio::test]
    async fn decisions_must_cover_exactly_the_pending_calls() {
        let f = fixture();
        let caller = actor();
        let chat = chat(&f, &caller).await;
        f.chats
            .prepare(&caller, chat.id(), send("Go"))
            .await
            .unwrap();
        let paused = pause(&f, &caller, &chat).await;
        let calls: Vec<_> = paused.calls().cloned().collect();
        let one = |call: &ToolCallPart| CallDecision {
            call_id: call.id,
            decision: Decision::Approve,
            scope: DecisionScope::Once,
            reason: None,
        };
        let cases = [
            vec![one(&calls[0])],
            vec![one(&calls[0]), one(&calls[0]), one(&calls[1])],
            vec![one(&calls[0]), one(&calls[1]), one(&calls[2])],
            Vec::new(),
        ];
        for decisions in cases {
            let input = TurnInput::Decide {
                message_id: paused.id(),
                decisions,
            };
            let error = f
                .chats
                .prepare(&caller, chat.id(), input)
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "Decide each pending tool call of this message exactly once."
            );
        }
        let mut long = one(&calls[1]);
        long.decision = Decision::Deny;
        long.reason = Some("x".repeat(501));
        let input = TurnInput::Decide {
            message_id: paused.id(),
            decisions: vec![one(&calls[0]), long],
        };
        let error = f
            .chats
            .prepare(&caller, chat.id(), input)
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "A reason must be at most 500 characters."
        );
        let stored = history(&f, &caller, &chat).await;
        assert_eq!(
            stored[1].status(),
            MessageStatus::AwaitingApproval,
            "nothing applied"
        );

        let unknown = TurnInput::Decide {
            message_id: MessageId::generate(),
            decisions: Vec::new(),
        };
        let error = f
            .chats
            .prepare(&caller, chat.id(), unknown)
            .await
            .unwrap_err();
        assert!(is_not_found(&error, "Message"));
        let user_message = TurnInput::Decide {
            message_id: stored[0].id(),
            decisions: Vec::new(),
        };
        let error = f
            .chats
            .prepare(&caller, chat.id(), user_message)
            .await
            .unwrap_err();
        assert_eq!(error.code(), NOT_AWAITING_APPROVAL);
    }

    #[tokio::test]
    async fn a_scheduled_run_starts_an_unread_chat_with_the_prompt() {
        let f = fixture();
        let caller = actor();
        let (model, _) = model(&f, &caller).await;
        let server = server(&f, &caller).await;
        let project = NewProject {
            name: "Ops".to_owned(),
            tool_server_ids: vec![server],
            ..NewProject::default()
        };
        let project = f.projects.create(&caller, project).await.unwrap();
        let input = NewSchedule {
            name: "Digest".to_owned(),
            cron: "0 9 * * *".to_owned(),
            timezone: "Pacific/Auckland".to_owned(),
            prompt: "Summarize the incidents.".to_owned(),
            project_id: Some(project.id()),
            model_id: Some(model),
            reasoning: Some(ReasoningLevel::High),
            on_ask: OnAsk::Pause,
            ..NewSchedule::default()
        };
        let schedule = f.schedules.create(&caller, input).await.unwrap();
        f.clock.advance(Duration::hours(12));
        let firing = f.schedules.run_now(&caller, schedule.id()).await.unwrap();

        let turn = f.chats.prepare_scheduled(firing).await.unwrap();
        assert_eq!(
            turn.mode,
            RunMode::Unattended {
                on_ask: OnAsk::Pause
            }
        );
        assert_eq!(
            turn.chat.title(),
            "Digest · 2026-01-02",
            "the date in the schedule's zone"
        );
        assert_eq!(turn.chat.schedule_id(), Some(schedule.id()));
        assert_eq!(turn.chat.project_id(), Some(project.id()));
        assert_eq!(turn.chat.model_id(), Some(model));
        assert_eq!(turn.chat.reasoning(), Some(ReasoningLevel::High));
        assert!(turn.chat.unread());
        assert_eq!(
            turn.schedule.as_ref().map(Schedule::id),
            Some(schedule.id())
        );
        assert_eq!(
            texts(&turn.history),
            [(Role::User, "Summarize the incidents.".to_owned())]
        );
        assert_eq!(turn.tools.tools.len(), 1, "the project's tools");
        assert_eq!(history(&f, &caller, &turn.chat).await, turn.history);
        let stored = f.chats.get(&caller, turn.chat.id()).await.unwrap();
        assert!(stored.unread());
    }

    #[tokio::test]
    async fn a_standalone_schedule_runs_without_tools_and_needs_its_model() {
        let f = fixture();
        let caller = actor();
        let (model, provider) = model(&f, &caller).await;
        server(&f, &caller).await;
        let input = NewSchedule {
            name: "Joke".to_owned(),
            cron: "0 9 * * *".to_owned(),
            timezone: "UTC".to_owned(),
            prompt: "Tell a joke.".to_owned(),
            instructions: "Keep it clean.".to_owned(),
            model_id: Some(model),
            ..NewSchedule::default()
        };
        let schedule = f.schedules.create(&caller, input).await.unwrap();
        let firing = f.schedules.run_now(&caller, schedule.id()).await.unwrap();
        let turn = f.chats.prepare_scheduled(firing).await.unwrap();
        assert!(turn.tools.tools.is_empty());
        assert!(turn.project.is_none());
        assert_eq!(
            turn.schedule.as_ref().map(Schedule::instructions),
            Some("Keep it clean."),
            "the standalone schedule's instructions travel with the turn"
        );
        assert_eq!(
            turn.mode,
            RunMode::Unattended {
                on_ask: OnAsk::Deny
            }
        );

        f.providers.delete(&caller, provider).await.unwrap();
        let firing = f.schedules.run_now(&caller, schedule.id()).await.unwrap();
        let chats_before = f
            .chats
            .list(&caller, ChatQuery::default())
            .await
            .unwrap()
            .items
            .len();
        let error = f.chats.prepare_scheduled(firing).await.unwrap_err();
        assert_eq!(error.to_string(), "Choose a model for this schedule.");
        let chats_after = f
            .chats
            .list(&caller, ChatQuery::default())
            .await
            .unwrap()
            .items
            .len();
        assert_eq!(chats_before, chats_after, "no empty chat for a failed run");
    }

    #[tokio::test]
    async fn chat_uploads_need_the_callers_chat() {
        let f = fixture();
        let alice = actor();
        let chat = chat(&f, &alice).await;
        let error = f
            .files
            .upload(
                &actor(),
                FileOwner::Chat(chat.id()),
                "a.png",
                Bytes::from_static(PNG),
            )
            .await
            .unwrap_err();
        assert!(is_not_found(&error, "Chat"));
        assert!(
            f.files
                .upload(
                    &alice,
                    FileOwner::Chat(chat.id()),
                    "a.png",
                    Bytes::from_static(PNG)
                )
                .await
                .is_ok()
        );
    }
}
