//! Projects: reusable chat contexts.

use chrono::{DateTime, Utc};

use crate::shared::builtin_tool::BuiltinTool;
use crate::shared::constants::NAME_MAX_CHARS;
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{ModelId, ProjectId, ToolServerId, UserId};
use crate::shared::reasoning::ReasoningLevel;
use crate::shared::text;

/// Longest icon, in characters. One emoji can be several code points (a family is seven).
const ICON_MAX_CHARS: usize = 10;

/// A project (`projects` plus its `project_tool_servers` rows): name, icon, instructions (the
/// system prompt), default model, reasoning level, enabled tools. Every chat in the project
/// inherits them at each turn. A prompt-only "agent" is a project with no files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Project {
    id: ProjectId,
    user_id: UserId,
    name: String,
    icon: String,
    instructions: String,
    default_model_id: Option<ModelId>,
    reasoning: Option<ReasoningLevel>,
    builtin_tools: Vec<BuiltinTool>,
    tool_server_ids: Vec<ToolServerId>,
    pinned: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Every field of a stored project, for [`Project::restore`].
#[derive(Clone, Debug)]
pub struct ProjectParts {
    pub id: ProjectId,
    pub user_id: UserId,
    pub name: String,
    pub icon: String,
    pub instructions: String,
    pub default_model_id: Option<ModelId>,
    pub reasoning: Option<ReasoningLevel>,
    pub builtin_tools: Vec<BuiltinTool>,
    pub tool_server_ids: Vec<ToolServerId>,
    pub pinned: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Project {
    /// A new, unpinned project with nothing but a name. Set the rest with the `with_*` methods.
    pub fn create(user_id: UserId, name: &str, now: DateTime<Utc>) -> DomainResult<Self> {
        Ok(Self {
            id: ProjectId::generate(),
            user_id,
            name: project_name(name)?,
            icon: String::new(),
            instructions: String::new(),
            default_model_id: None,
            reasoning: None,
            builtin_tools: Vec::new(),
            tool_server_ids: Vec::new(),
            pinned: false,
            created_at: now,
            updated_at: now,
        })
    }

    /// A row read back from storage.
    pub fn restore(parts: ProjectParts) -> Self {
        Self {
            id: parts.id,
            user_id: parts.user_id,
            name: parts.name,
            icon: parts.icon,
            instructions: parts.instructions,
            default_model_id: parts.default_model_id,
            reasoning: parts.reasoning,
            builtin_tools: parts.builtin_tools,
            tool_server_ids: parts.tool_server_ids,
            pinned: parts.pinned,
            created_at: parts.created_at,
            updated_at: parts.updated_at,
        }
    }

    pub fn with_name(mut self, name: &str) -> DomainResult<Self> {
        self.name = project_name(name)?;
        Ok(self)
    }

    /// Empty, or one emoji.
    pub fn with_icon(mut self, icon: &str) -> DomainResult<Self> {
        self.icon = project_icon(icon)?;
        Ok(self)
    }

    /// The project's system prompt (trimmed).
    #[must_use]
    pub fn with_instructions(mut self, instructions: &str) -> Self {
        self.instructions = text::trimmed(instructions);
        self
    }

    /// Check that the model belongs to the owner before calling this.
    #[must_use]
    pub fn with_default_model(mut self, model: Option<ModelId>) -> Self {
        self.default_model_id = model;
        self
    }

    /// `None` uses the model default.
    #[must_use]
    pub fn with_reasoning(mut self, reasoning: Option<ReasoningLevel>) -> Self {
        self.reasoning = reasoning;
        self
    }

    #[must_use]
    pub fn with_builtin_tools(mut self, tools: Vec<BuiltinTool>) -> Self {
        self.builtin_tools = BuiltinTool::normalize(tools);
        self
    }

    /// Unique, in the given order. Check that every server belongs to the owner first.
    #[must_use]
    pub fn with_tool_servers(mut self, servers: Vec<ToolServerId>) -> Self {
        let mut unique = Vec::with_capacity(servers.len());
        for server in servers {
            if !unique.contains(&server) {
                unique.push(server);
            }
        }
        self.tool_server_ids = unique;
        self
    }

    #[must_use]
    pub fn with_pinned(mut self, pinned: bool) -> Self {
        self.pinned = pinned;
        self
    }

    #[must_use]
    pub fn touched(mut self, now: DateTime<Utc>) -> Self {
        self.updated_at = now;
        self
    }

    pub fn id(&self) -> ProjectId {
        self.id
    }

    pub fn user_id(&self) -> UserId {
        self.user_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn icon(&self) -> &str {
        &self.icon
    }

    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    pub fn default_model_id(&self) -> Option<ModelId> {
        self.default_model_id
    }

    pub fn reasoning(&self) -> Option<ReasoningLevel> {
        self.reasoning
    }

    pub fn builtin_tools(&self) -> &[BuiltinTool] {
        &self.builtin_tools
    }

    pub fn tool_server_ids(&self) -> &[ToolServerId] {
        &self.tool_server_ids
    }

    pub fn pinned(&self) -> bool {
        self.pinned
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

fn project_name(name: &str) -> DomainResult<String> {
    text::name(name, "Project name", "name", NAME_MAX_CHARS)
}

/// An icon is empty or one emoji. Without a Unicode segmentation table this accepts a short run
/// of characters with no whitespace or control characters and at least one non-ASCII character,
/// which rejects plain words while allowing ZWJ sequences, flags, and keycaps.
fn project_icon(icon: &str) -> DomainResult<String> {
    let icon = icon.trim();
    if icon.is_empty() {
        return Ok(String::new());
    }
    let valid = icon.chars().count() <= ICON_MAX_CHARS
        && !icon
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        && !icon.is_ascii();
    if valid {
        Ok(icon.to_owned())
    } else {
        Err(DomainError::validation("Choose one emoji as the icon.").with_field("icon"))
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use uuid::Uuid;

    use super::*;

    fn project() -> Project {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Project::create(UserId::new(Uuid::now_v7()), "Work", now).unwrap()
    }

    #[test]
    fn starts_with_a_name_only() {
        let project = project();
        assert_eq!(project.name(), "Work");
        assert_eq!(project.icon(), "");
        assert_eq!(project.instructions(), "");
        assert_eq!(project.default_model_id(), None);
        assert!(project.builtin_tools().is_empty());
        assert!(project.tool_server_ids().is_empty());
        assert!(!project.pinned());
    }

    #[test]
    fn accepts_one_emoji_as_the_icon() {
        for icon in ["📁", "👨‍👩‍👧‍👦", "🇩🇪", "1️⃣", " 🚀 ", ""] {
            assert!(project().with_icon(icon).is_ok(), "{icon}");
        }
        assert_eq!(project().with_icon(" 🚀 ").unwrap().icon(), "🚀");
    }

    #[test]
    fn rejects_words_and_long_icons() {
        for icon in ["ab", "x", "🚀 🚀", "🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀", "a\u{0007}é"]
        {
            let error = project().with_icon(icon).unwrap_err();
            assert_eq!(error.to_string(), "Choose one emoji as the icon.", "{icon}");
        }
    }

    #[test]
    fn keeps_tool_servers_unique_in_order() {
        let first = ToolServerId::generate();
        let second = ToolServerId::generate();
        let project = project().with_tool_servers(vec![second, first, second]);
        assert_eq!(project.tool_server_ids(), [second, first]);
    }

    #[test]
    fn trims_instructions() {
        let project = project().with_instructions("\n You review contracts.\n");
        assert_eq!(project.instructions(), "You review contracts.");
    }
}
