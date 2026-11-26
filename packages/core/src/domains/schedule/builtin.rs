//! The built-in schedule tools: `schedule_prompt`, `list_schedules`, and `cancel_schedule`.
//!
//! They are offered only to interactive runs whose chat turned on the `schedules` built-in tool
//! group; unattended runs never see them, so a run cannot create more schedules. Their handlers
//! are `ScheduleService::{schedule_prompt, list_schedules, cancel_schedule}`.

use serde_json::{Map, Value, json};

use crate::domains::tool::{ToolAlias, ToolSpec};
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{ChatId, ModelId, ProjectId, ScheduleId};

/// One of the built-in schedule tools.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ScheduleTool {
    /// Creates a schedule. Always asks.
    Prompt,
    /// Lists the schedules the assistant created here. Runs without asking.
    List,
    /// Deletes one of them. Always asks.
    Cancel,
}

impl ScheduleTool {
    pub const ALL: [Self; 3] = [Self::Prompt, Self::List, Self::Cancel];

    /// The reserved name (no `__`, so it never collides with a server tool's alias).
    pub fn name(self) -> &'static str {
        match self {
            Self::Prompt => "schedule_prompt",
            Self::List => "list_schedules",
            Self::Cancel => "cancel_schedule",
        }
    }

    pub fn find(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.name() == name)
    }

    /// `schedule_prompt` and `cancel_schedule` always ask, whatever any mode says.
    pub fn always_asks(self) -> bool {
        matches!(self, Self::Prompt | Self::Cancel)
    }

    /// What the model is offered.
    pub fn spec(self) -> ToolSpec {
        let (description, input_schema) = match self {
            Self::Prompt => (
                "Schedule a prompt to run at set times. Each run starts a new chat with the same \
                 model and project. The user approves every new schedule.",
                json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "A short name for the schedule."},
                        "cron": {
                            "type": "string",
                            "description": "A 5-field cron rule: minute, hour, day of month, month, day of week. Runs must be at least 15 minutes apart."
                        },
                        "timezone": {"type": "string", "description": "An IANA time zone, for example Europe/Berlin."},
                        "prompt": {"type": "string", "description": "The prompt each run sends."}
                    },
                    "required": ["name", "cron", "timezone", "prompt"],
                    "additionalProperties": false
                }),
            ),
            Self::List => (
                "List the schedules you created in this project, or from this chat outside a \
                 project.",
                json!({"type": "object", "properties": {}, "additionalProperties": false}),
            ),
            Self::Cancel => (
                "Delete a schedule you created, by its id from list_schedules. The user approves \
                 it first.",
                json!({
                    "type": "object",
                    "properties": {
                        "id": {"type": "string", "description": "The schedule id from list_schedules."}
                    },
                    "required": ["id"],
                    "additionalProperties": false
                }),
            ),
        };
        ToolSpec {
            alias: ToolAlias::builtin(self.name()),
            description: description.to_owned(),
            input_schema,
        }
    }
}

/// The chat a built-in tool call comes from. The service fills the user, project, model, and
/// source chat of a new schedule from it, never from the model's arguments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolOrigin {
    pub chat_id: ChatId,
    pub project_id: Option<ProjectId>,
    pub model_id: Option<ModelId>,
    /// The chat instructions, copied into a standalone schedule.
    pub instructions: String,
}

/// The `schedule_prompt` input: only `name`, `cron`, `timezone`, and `prompt`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchedulePrompt {
    pub name: String,
    pub cron: String,
    pub timezone: String,
    pub prompt: String,
}

impl SchedulePrompt {
    const FIELDS: [&str; 4] = ["name", "cron", "timezone", "prompt"];

    /// Reads the model's arguments. Any other field, or a missing or non-text one, is an error
    /// the model sees.
    pub fn parse(arguments: &Value) -> DomainResult<Self> {
        let object = object(arguments, ScheduleTool::Prompt)?;
        if let Some(extra) = object
            .keys()
            .find(|key| !Self::FIELDS.contains(&key.as_str()))
        {
            return Err(DomainError::validation(format!(
                "schedule_prompt takes only name, cron, timezone, and prompt, not {extra}."
            )));
        }
        let text = |field: &str| -> DomainResult<String> {
            match object.get(field) {
                Some(Value::String(value)) => Ok(value.clone()),
                _ => Err(DomainError::validation(format!(
                    "schedule_prompt needs {field} as text."
                ))
                .with_field(field)),
            }
        };
        Ok(Self {
            name: text("name")?,
            cron: text("cron")?,
            timezone: text("timezone")?,
            prompt: text("prompt")?,
        })
    }
}

/// The schedule id of a `cancel_schedule` call.
pub fn cancel_target(arguments: &Value) -> DomainResult<ScheduleId> {
    let object = object(arguments, ScheduleTool::Cancel)?;
    object
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| id.trim().parse().ok())
        .ok_or_else(|| {
            DomainError::validation(
                "cancel_schedule needs the id of a schedule from list_schedules.",
            )
            .with_field("id")
        })
}

fn object(arguments: &Value, tool: ScheduleTool) -> DomainResult<&Map<String, Value>> {
    arguments.as_object().ok_or_else(|| {
        DomainError::validation(format!("{} needs a JSON object of arguments.", tool.name()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_reserved_and_two_always_ask() {
        for tool in ScheduleTool::ALL {
            assert_eq!(ScheduleTool::find(tool.name()), Some(tool));
            let spec = tool.spec();
            assert!(spec.alias.is_builtin());
            assert_eq!(spec.alias.as_str(), tool.name());
            assert_eq!(spec.input_schema["type"], "object");
        }
        assert!(ScheduleTool::Prompt.always_asks());
        assert!(ScheduleTool::Cancel.always_asks());
        assert!(!ScheduleTool::List.always_asks());
        assert_eq!(ScheduleTool::find("github__search"), None);
    }

    #[test]
    fn the_prompt_input_has_exactly_four_fields() {
        let spec = ScheduleTool::Prompt.spec();
        assert_eq!(
            spec.input_schema["required"],
            json!(["name", "cron", "timezone", "prompt"])
        );
        let input = SchedulePrompt::parse(&json!({
            "name": "Digest", "cron": "0 9 * * *", "timezone": "UTC", "prompt": "News?"
        }))
        .unwrap();
        assert_eq!(input.name, "Digest");
        assert_eq!(input.prompt, "News?");
    }

    #[test]
    fn rejects_other_fields_and_wrong_types() {
        let extra = json!({
            "name": "Digest", "cron": "0 9 * * *", "timezone": "UTC", "prompt": "News?",
            "project": "other"
        });
        assert_eq!(
            SchedulePrompt::parse(&extra).unwrap_err().to_string(),
            "schedule_prompt takes only name, cron, timezone, and prompt, not project."
        );
        let missing = json!({"name": "Digest", "cron": "0 9 * * *", "timezone": "UTC"});
        assert_eq!(
            SchedulePrompt::parse(&missing).unwrap_err().to_string(),
            "schedule_prompt needs prompt as text."
        );
        let number = json!({"name": 1, "cron": "0 9 * * *", "timezone": "UTC", "prompt": "x"});
        assert!(SchedulePrompt::parse(&number).is_err());
        assert_eq!(
            SchedulePrompt::parse(&json!([1])).unwrap_err().to_string(),
            "schedule_prompt needs a JSON object of arguments."
        );
    }

    #[test]
    fn reads_the_cancel_target() {
        let id = ScheduleId::generate();
        assert_eq!(cancel_target(&json!({"id": id.to_string()})).unwrap(), id);
        for bad in [
            json!({}),
            json!({"id": "nope"}),
            json!({"id": 3}),
            json!("x"),
        ] {
            assert!(cancel_target(&bad).is_err(), "{bad}");
        }
    }
}
