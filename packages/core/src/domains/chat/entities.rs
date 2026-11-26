//! Chats: a conversation with one model, in a project or standalone.

use chrono::{DateTime, Utc};

use crate::domains::project::Project;
use crate::domains::schedule::ToolOrigin;
use crate::domains::tool::{ServerOverride, ToolAlias, ToolSelection};
use crate::shared::builtin_tool::BuiltinTool;
use crate::shared::constants::{NAME_MAX_CHARS, TITLE_MAX_CHARS};
use crate::shared::errors::DomainResult;
use crate::shared::ids::{ChatId, ModelId, ProjectId, ScheduleId, UserId};
use crate::shared::reasoning::ReasoningLevel;
use crate::shared::text;

/// A chat (`chats` plus its `chat_tool_servers` rows). It never loads its messages.
///
/// `updated_at` moves with every message write, not with settings changes, so renaming or
/// pinning a chat neither reorders the list nor marks it unread.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chat {
    id: ChatId,
    user_id: UserId,
    project_id: Option<ProjectId>,
    model_id: Option<ModelId>,
    schedule_id: Option<ScheduleId>,
    title: String,
    instructions: String,
    reasoning: Option<ReasoningLevel>,
    builtin_tools: Option<Vec<BuiltinTool>>,
    approved_tools: Vec<ToolAlias>,
    tool_overrides: Vec<ServerOverride>,
    pinned: bool,
    voice_replies: bool,
    last_read_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Every field of a stored chat, for [`Chat::restore`].
#[derive(Clone, Debug)]
pub struct ChatParts {
    pub id: ChatId,
    pub user_id: UserId,
    pub project_id: Option<ProjectId>,
    pub model_id: Option<ModelId>,
    pub schedule_id: Option<ScheduleId>,
    pub title: String,
    pub instructions: String,
    pub reasoning: Option<ReasoningLevel>,
    pub builtin_tools: Option<Vec<BuiltinTool>>,
    pub approved_tools: Vec<ToolAlias>,
    pub tool_overrides: Vec<ServerOverride>,
    pub pinned: bool,
    pub voice_replies: bool,
    pub last_read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Chat {
    /// A new, untitled, standalone chat, read at `now` (the user is looking at it). The first
    /// user message gives it a title.
    pub fn create(user_id: UserId, now: DateTime<Utc>) -> Self {
        Self {
            id: ChatId::generate(),
            user_id,
            project_id: None,
            model_id: None,
            schedule_id: None,
            title: String::new(),
            instructions: String::new(),
            reasoning: None,
            builtin_tools: None,
            approved_tools: Vec::new(),
            tool_overrides: Vec::new(),
            pinned: false,
            voice_replies: false,
            last_read_at: Some(now),
            created_at: now,
            updated_at: now,
        }
    }

    /// The chat of one run of `schedule`, titled `<schedule name> · <local date>`, unread.
    pub fn scheduled(
        user_id: UserId,
        schedule: ScheduleId,
        title: String,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            schedule_id: Some(schedule),
            title,
            last_read_at: None,
            ..Self::create(user_id, now)
        }
    }

    /// A row read back from storage.
    pub fn restore(parts: ChatParts) -> Self {
        Self {
            id: parts.id,
            user_id: parts.user_id,
            project_id: parts.project_id,
            model_id: parts.model_id,
            schedule_id: parts.schedule_id,
            title: parts.title,
            instructions: parts.instructions,
            reasoning: parts.reasoning,
            builtin_tools: parts.builtin_tools,
            approved_tools: parts.approved_tools,
            tool_overrides: parts.tool_overrides,
            pinned: parts.pinned,
            voice_replies: parts.voice_replies,
            last_read_at: parts.last_read_at,
            created_at: parts.created_at,
            updated_at: parts.updated_at,
        }
    }

    /// Every field, for building a changed copy with [`Chat::restore`].
    pub fn to_parts(&self) -> ChatParts {
        ChatParts {
            id: self.id,
            user_id: self.user_id,
            project_id: self.project_id,
            model_id: self.model_id,
            schedule_id: self.schedule_id,
            title: self.title.clone(),
            instructions: self.instructions.clone(),
            reasoning: self.reasoning,
            builtin_tools: self.builtin_tools.clone(),
            approved_tools: self.approved_tools.clone(),
            tool_overrides: self.tool_overrides.clone(),
            pinned: self.pinned,
            voice_replies: self.voice_replies,
            last_read_at: self.last_read_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }

    /// A rename.
    pub fn with_title(mut self, title: &str) -> DomainResult<Self> {
        self.title = text::name(title, "Title", "title", NAME_MAX_CHARS)?;
        Ok(self)
    }

    /// Titles an untitled chat after its first user message: whitespace collapsed, cut to
    /// `TITLE_MAX_CHARS`. A chat with a title keeps it.
    #[must_use]
    pub fn titled_by(mut self, text: &str) -> Self {
        if self.title.is_empty() {
            self.title = title(text);
        }
        self
    }

    /// Check that the project belongs to the owner first.
    #[must_use]
    pub fn with_project(mut self, project: Option<ProjectId>) -> Self {
        self.project_id = project;
        self
    }

    /// Check that the model belongs to the owner first.
    #[must_use]
    pub fn with_model(mut self, model: Option<ModelId>) -> Self {
        self.model_id = model;
        self
    }

    /// Appended after the project's instructions; trimmed.
    #[must_use]
    pub fn with_instructions(mut self, instructions: &str) -> Self {
        self.instructions = text::trimmed(instructions);
        self
    }

    /// `None` inherits the project, then the model default.
    #[must_use]
    pub fn with_reasoning(mut self, reasoning: Option<ReasoningLevel>) -> Self {
        self.reasoning = reasoning;
        self
    }

    /// `None` inherits the project's built-in tools (none when standalone).
    #[must_use]
    pub fn with_builtin_tools(mut self, tools: Option<Vec<BuiltinTool>>) -> Self {
        self.builtin_tools = tools.map(BuiltinTool::normalize);
        self
    }

    /// One override per server; the last one given wins. Check that every server belongs to
    /// the owner first.
    #[must_use]
    pub fn with_tool_overrides(mut self, overrides: Vec<ServerOverride>) -> Self {
        let mut unique: Vec<ServerOverride> = Vec::with_capacity(overrides.len());
        for row in overrides {
            match unique
                .iter_mut()
                .find(|kept| kept.server_id == row.server_id)
            {
                Some(kept) => kept.enabled = row.enabled,
                None => unique.push(row),
            }
        }
        self.tool_overrides = unique;
        self
    }

    /// "Approve for this chat": later calls to `alias` in this chat run without asking.
    #[must_use]
    pub fn approving(mut self, alias: ToolAlias) -> Self {
        if !self.approved_tools.contains(&alias) {
            self.approved_tools.push(alias);
        }
        self
    }

    #[must_use]
    pub fn with_pinned(mut self, pinned: bool) -> Self {
        self.pinned = pinned;
        self
    }

    /// "Speak replies".
    #[must_use]
    pub fn with_voice_replies(mut self, voice_replies: bool) -> Self {
        self.voice_replies = voice_replies;
        self
    }

    /// The tool servers this chat selects in `project`.
    pub fn tool_selection(&self, project: Option<&Project>) -> ToolSelection {
        ToolSelection {
            project: project
                .map(|project| project.tool_server_ids().to_vec())
                .unwrap_or_default(),
            overrides: self.tool_overrides.clone(),
        }
    }

    /// The chat's built-in tools, else the project's, else none.
    pub fn effective_builtin_tools(&self, project: Option<&Project>) -> Vec<BuiltinTool> {
        match (&self.builtin_tools, project) {
            (Some(tools), _) => tools.clone(),
            (None, Some(project)) => project.builtin_tools().to_vec(),
            (None, None) => Vec::new(),
        }
    }

    /// The chat's reasoning level, else the project's; `None` is the model default.
    pub fn effective_reasoning(&self, project: Option<&Project>) -> Option<ReasoningLevel> {
        self.reasoning
            .or_else(|| project.and_then(Project::reasoning))
    }

    /// What the built-in schedule tools fill a new schedule from.
    pub fn origin(&self) -> ToolOrigin {
        ToolOrigin {
            chat_id: self.id,
            project_id: self.project_id,
            model_id: self.model_id,
            instructions: self.instructions.clone(),
        }
    }

    /// Changed since the user last opened it.
    pub fn unread(&self) -> bool {
        self.last_read_at.is_none_or(|read| self.updated_at > read)
    }

    pub fn id(&self) -> ChatId {
        self.id
    }

    pub fn user_id(&self) -> UserId {
        self.user_id
    }

    pub fn project_id(&self) -> Option<ProjectId> {
        self.project_id
    }

    pub fn model_id(&self) -> Option<ModelId> {
        self.model_id
    }

    /// The schedule whose run created this chat.
    pub fn schedule_id(&self) -> Option<ScheduleId> {
        self.schedule_id
    }

    /// Empty until the first user message.
    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    pub fn reasoning(&self) -> Option<ReasoningLevel> {
        self.reasoning
    }

    pub fn builtin_tools(&self) -> Option<&[BuiltinTool]> {
        self.builtin_tools.as_deref()
    }

    pub fn approved_tools(&self) -> &[ToolAlias] {
        &self.approved_tools
    }

    pub fn tool_overrides(&self) -> &[ServerOverride] {
        &self.tool_overrides
    }

    pub fn pinned(&self) -> bool {
        self.pinned
    }

    pub fn voice_replies(&self) -> bool {
        self.voice_replies
    }

    pub fn last_read_at(&self) -> Option<DateTime<Utc>> {
        self.last_read_at
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

/// A chat title from the first user message.
fn title(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = collapsed.chars().take(TITLE_MAX_CHARS).collect();
    cut.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};
    use uuid::Uuid;

    use super::*;
    use crate::shared::ids::ToolServerId;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    fn chat() -> Chat {
        Chat::create(UserId::new(Uuid::now_v7()), now())
    }

    fn project(user: UserId) -> Project {
        Project::create(user, "Work", now()).unwrap()
    }

    #[test]
    fn titles_by_the_first_user_message_only() {
        let long = format!("Plan   my\nweek {}", "x".repeat(100));
        let chat = chat().titled_by(&long);
        assert_eq!(chat.title().chars().count(), TITLE_MAX_CHARS);
        assert!(chat.title().starts_with("Plan my week xxx"));
        assert_eq!(chat.clone().titled_by("Other").title(), chat.title());
        assert_eq!(super::title("  short  "), "short");
        assert_eq!(
            super::title(&format!("{} y", "a".repeat(59))),
            "a".repeat(59)
        );
    }

    #[test]
    fn renames_with_validation() {
        assert_eq!(chat().with_title(" Trip ").unwrap().title(), "Trip");
        assert_eq!(
            chat().with_title(" ").unwrap_err().to_string(),
            "Title is required."
        );
    }

    #[test]
    fn keeps_one_override_per_server() {
        let first = ToolServerId::generate();
        let second = ToolServerId::generate();
        let chat = chat().with_tool_overrides(vec![
            ServerOverride {
                server_id: first,
                enabled: true,
            },
            ServerOverride {
                server_id: second,
                enabled: true,
            },
            ServerOverride {
                server_id: first,
                enabled: false,
            },
        ]);
        assert_eq!(
            chat.tool_overrides(),
            [
                ServerOverride {
                    server_id: first,
                    enabled: false,
                },
                ServerOverride {
                    server_id: second,
                    enabled: true,
                },
            ]
        );
    }

    #[test]
    fn inherits_tools_and_reasoning_from_the_project() {
        let chat = chat();
        let server = ToolServerId::generate();
        let project = project(chat.user_id())
            .with_builtin_tools(vec![BuiltinTool::Schedules])
            .with_tool_servers(vec![server])
            .with_reasoning(Some(ReasoningLevel::High));
        assert_eq!(
            chat.effective_builtin_tools(Some(&project)),
            [BuiltinTool::Schedules]
        );
        assert!(chat.effective_builtin_tools(None).is_empty());
        assert_eq!(
            chat.effective_reasoning(Some(&project)),
            Some(ReasoningLevel::High)
        );
        assert_eq!(chat.tool_selection(Some(&project)).servers(), [server]);

        let chat = chat
            .with_builtin_tools(Some(Vec::new()))
            .with_reasoning(Some(ReasoningLevel::Off));
        assert!(
            chat.effective_builtin_tools(Some(&project)).is_empty(),
            "chat wins"
        );
        assert_eq!(
            chat.effective_reasoning(Some(&project)),
            Some(ReasoningLevel::Off)
        );
    }

    #[test]
    fn approves_each_alias_once() {
        let alias = ToolAlias::builtin("list_schedules");
        let chat = chat().approving(alias.clone()).approving(alias.clone());
        assert_eq!(chat.approved_tools(), [alias]);
    }

    #[test]
    fn new_chats_are_read_and_scheduled_runs_unread() {
        let chat = chat();
        assert!(!chat.unread());
        let changed = Chat::restore(ChatParts {
            updated_at: now() + Duration::seconds(1),
            ..chat.to_parts()
        });
        assert!(changed.unread());
        let run = Chat::scheduled(
            chat.user_id(),
            ScheduleId::generate(),
            "Digest · 2026-01-01".to_owned(),
            now(),
        );
        assert!(run.unread());
        assert_eq!(run.title(), "Digest · 2026-01-01");
    }

    #[test]
    fn exposes_the_origin_for_built_in_tools() {
        let model = ModelId::generate();
        let chat = chat()
            .with_model(Some(model))
            .with_instructions(" Be brief. ");
        let origin = chat.origin();
        assert_eq!(origin.chat_id, chat.id());
        assert_eq!(origin.model_id, Some(model));
        assert_eq!(origin.project_id, None);
        assert_eq!(origin.instructions, "Be brief.");
    }
}
