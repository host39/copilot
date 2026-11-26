//! Built-in tool groups a project or chat can turn on.

use std::fmt;
use std::str::FromStr;

/// A group of tools Copilot implements itself (`builtin_tools` on projects and chats).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum BuiltinTool {
    /// `schedule_prompt`, `list_schedules`, and `cancel_schedule`; off by default, offered only
    /// to interactive runs.
    Schedules,
}

impl BuiltinTool {
    pub const ALL: [Self; 1] = [Self::Schedules];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Schedules => "schedules",
        }
    }

    /// Sorted and without duplicates, the form projects and chats store.
    pub fn normalize(mut tools: Vec<Self>) -> Vec<Self> {
        tools.sort_unstable();
        tools.dedup();
        tools
    }
}

impl fmt::Display for BuiltinTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for BuiltinTool {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|tool| tool.as_str() == value)
            .ok_or_else(|| value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_normalizes() {
        assert_eq!(
            "schedules".parse::<BuiltinTool>(),
            Ok(BuiltinTool::Schedules)
        );
        assert!("search".parse::<BuiltinTool>().is_err());
        assert_eq!(
            BuiltinTool::normalize(vec![BuiltinTool::Schedules, BuiltinTool::Schedules]),
            [BuiltinTool::Schedules]
        );
    }
}
