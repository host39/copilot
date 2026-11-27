//! Which tools a turn gets: effective servers, aliases, and changed-tool detection.

use std::collections::{BTreeSet, HashMap};

use serde_json::Value;

use crate::domains::tool::entities::ToolServer;
use crate::domains::tool::values::{ToolAlias, ToolMode};
use crate::ports::tool_host::RemoteTool;
use crate::shared::ids::ToolServerId;

/// A chat's tool server override (`chat_tool_servers`): `enabled = true` adds the server,
/// `false` removes one of the project's servers in this chat.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ServerOverride {
    pub server_id: ToolServerId,
    pub enabled: bool,
}

/// The tool servers a chat selects: its project's servers plus its own overrides.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolSelection {
    pub project: Vec<ToolServerId>,
    pub overrides: Vec<ServerOverride>,
}

impl ToolSelection {
    /// Project servers + overrides with `enabled = true` - overrides with `enabled = false`, in
    /// that order and without duplicates. Whether each server is enabled is checked later.
    pub fn servers(&self) -> Vec<ToolServerId> {
        let removed: Vec<ToolServerId> = self
            .overrides
            .iter()
            .filter(|row| !row.enabled)
            .map(|row| row.server_id)
            .collect();
        let added = self
            .overrides
            .iter()
            .filter(|row| row.enabled)
            .map(|row| row.server_id);
        let mut servers = Vec::new();
        for id in self.project.iter().copied().chain(added) {
            if !removed.contains(&id) && !servers.contains(&id) {
                servers.push(id);
            }
        }
        servers
    }
}

/// What the model is offered for a tool: the alias and the definition.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolSpec {
    pub alias: ToolAlias,
    pub description: String,
    pub input_schema: Value,
}

/// A server tool a turn may use, with what the approval policy needs.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectiveTool {
    pub alias: ToolAlias,
    pub server_id: ToolServerId,
    pub server_name: String,
    /// The original tool name, never the alias.
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    pub input_schema: Value,
    /// `ask` or `auto` (`off` tools are never offered).
    pub mode: ToolMode,
    /// The server's tool list changed and was not reviewed yet: every call asks.
    pub review_required: bool,
}

impl EffectiveTool {
    pub fn spec(&self) -> ToolSpec {
        ToolSpec {
            alias: self.alias.clone(),
            description: self.description.clone(),
            input_schema: self.input_schema.clone(),
        }
    }
}

/// The server tools of a turn, sorted by alias, plus the selected servers whose tools could not
/// be listed (the UI names them).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectiveTools {
    pub tools: Vec<EffectiveTool>,
    pub unavailable: Vec<ToolServerId>,
}

/// The offered tools of one server from `tools` (its cached list): `off` tools dropped, each
/// tool aliased. When two names normalize to the same alias, every one after the first (in name
/// order) takes the hashed alias.
pub fn offered(server: &ToolServer, tools: &[RemoteTool]) -> Vec<EffectiveTool> {
    let mut sorted: Vec<&RemoteTool> = tools.iter().collect();
    sorted.sort_by(|left, right| left.name.cmp(&right.name));
    let mut used = BTreeSet::new();
    let mut offered = Vec::new();
    for tool in sorted {
        let mode = server.mode(&tool.name);
        if mode == ToolMode::Off {
            continue;
        }
        let mut alias = ToolAlias::new(server.slug(), &tool.name);
        if used.contains(&alias) {
            alias = ToolAlias::hashed(server.slug(), &tool.name);
        }
        used.insert(alias.clone());
        offered.push(EffectiveTool {
            alias,
            server_id: server.id(),
            server_name: server.name().to_owned(),
            name: tool.name.clone(),
            title: tool.title.clone(),
            description: tool.description.clone(),
            input_schema: tool.input_schema.clone(),
            mode,
            review_required: server.review_required(),
        });
    }
    offered
}

/// How a refreshed tool list differs from the cached one, by tool name. A change of a tool's
/// name, description, or input schema puts the server back to `ask` until the user reviews it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolChanges {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// Same name, new description or input schema.
    pub changed: Vec<String>,
}

impl ToolChanges {
    /// `None` for `before` is the first listing, which is not a change.
    pub fn between(before: Option<&[RemoteTool]>, after: &[RemoteTool]) -> Self {
        let Some(before) = before else {
            return Self::default();
        };
        // Indexed once, so the comparison is linear in the number of tools.
        let (old, new) = (index(before), index(after));
        let mut changes = Self::default();
        for tool in after {
            match old.get(tool.name.as_str()) {
                None => changes.added.push(tool.name.clone()),
                Some(previous)
                    if previous.description != tool.description
                        || previous.input_schema != tool.input_schema =>
                {
                    changes.changed.push(tool.name.clone());
                }
                Some(_) => {}
            }
        }
        for tool in before {
            if !new.contains_key(tool.name.as_str()) {
                changes.removed.push(tool.name.clone());
            }
        }
        changes.added.sort();
        changes.removed.sort();
        changes.changed.sort();
        changes
    }

    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

fn index(tools: &[RemoteTool]) -> HashMap<&str, &RemoteTool> {
    tools
        .iter()
        .map(|tool| (tool.name.as_str(), tool))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::domains::tool::values::{ApprovalMode, Slug};
    use crate::shared::base_url::BaseUrl;
    use crate::shared::ids::UserId;
    use crate::shared::net::NetworkScope;

    fn tool(name: &str, description: &str) -> RemoteTool {
        RemoteTool {
            name: name.to_owned(),
            title: None,
            description: description.to_owned(),
            input_schema: json!({"type": "object"}),
            read_only: false,
        }
    }

    fn server() -> ToolServer {
        let url = BaseUrl::parse("https://mcp.example.com", NetworkScope::Public).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        ToolServer::create(
            UserId::new(Uuid::now_v7()),
            "Files",
            Slug::parse("fs").unwrap(),
            url,
            now,
        )
        .unwrap()
    }

    #[test]
    fn effective_servers_are_project_plus_added_minus_removed() {
        let [a, b, c, d] = [(); 4].map(|()| ToolServerId::generate());
        let selection = ToolSelection {
            project: vec![a, b, a],
            overrides: vec![
                ServerOverride {
                    server_id: b,
                    enabled: false,
                },
                ServerOverride {
                    server_id: c,
                    enabled: true,
                },
                ServerOverride {
                    server_id: a,
                    enabled: true,
                },
                ServerOverride {
                    server_id: d,
                    enabled: false,
                },
            ],
        };
        assert_eq!(selection.servers(), [a, c]);
        assert!(ToolSelection::default().servers().is_empty());
    }

    #[test]
    fn drops_off_tools_and_keeps_modes() {
        let server = server()
            .with_approval(ApprovalMode::Auto)
            .with_tool_modes(BTreeMap::from([
                ("delete".to_owned(), ToolMode::Off),
                ("write".to_owned(), ToolMode::Ask),
            ]));
        let tools = [tool("write", ""), tool("delete", ""), tool("read", "")];
        let offered = offered(&server, &tools);
        let names: Vec<(&str, &str, ToolMode)> = offered
            .iter()
            .map(|tool| (tool.alias.as_str(), tool.name.as_str(), tool.mode))
            .collect();
        assert_eq!(
            names,
            [
                ("fs__read", "read", ToolMode::Auto),
                ("fs__write", "write", ToolMode::Ask),
            ]
        );
        assert_eq!(offered[0].server_name, "Files");
        assert_eq!(offered[0].spec().alias.as_str(), "fs__read");
    }

    #[test]
    fn hashes_aliases_that_collide_after_normalizing() {
        let offered = offered(&server(), &[tool("a_b", ""), tool("a.b", "")]);
        assert_eq!(offered.len(), 2);
        assert_eq!(
            offered[0].name, "a.b",
            "name order decides who keeps the plain alias"
        );
        assert_eq!(offered[0].alias.as_str(), "fs__a_b");
        assert_ne!(offered[1].alias, offered[0].alias);
        assert!(offered[1].alias.as_str().starts_with("fs__a_b_"));
    }

    #[test]
    fn carries_the_review_flag() {
        let listing = crate::ports::tool_host::ToolListing {
            tools: vec![tool("read", "")],
            protocol_version: "2025-11-25".to_owned(),
            ttl: None,
        };
        let server = server().with_listing(&listing, true, Utc::now());
        assert!(offered(&server, &listing.tools)[0].review_required);
    }

    #[test]
    fn the_first_listing_is_not_a_change() {
        assert!(ToolChanges::between(None, &[tool("read", "")]).is_empty());
    }

    #[test]
    fn detects_added_removed_and_changed_tools() {
        let before = [
            tool("read", "Read a file"),
            tool("write", "Write"),
            tool("old", ""),
        ];
        let mut schema_changed = tool("write", "Write");
        schema_changed.input_schema = json!({"type": "object", "required": ["path"]});
        let after = [tool("read", "Read a file"), schema_changed, tool("new", "")];
        let changes = ToolChanges::between(Some(&before), &after);
        assert_eq!(changes.added, ["new"]);
        assert_eq!(changes.removed, ["old"]);
        assert_eq!(changes.changed, ["write"]);

        let reworded = [
            tool("read", "Read any file"),
            tool("write", "Write"),
            tool("old", ""),
        ];
        let changes = ToolChanges::between(Some(&before), &reworded);
        assert_eq!(changes.changed, ["read"]);
        assert!(ToolChanges::between(Some(&before), &before).is_empty());
    }

    /// The comparison indexes both lists, so tens of thousands of tools take milliseconds
    /// (a search per tool took over a minute at 90,000).
    #[test]
    fn compares_long_lists_in_linear_time() {
        let before: Vec<RemoteTool> = (0..50_000)
            .map(|index| tool(&format!("tool{index}"), "old"))
            .collect();
        let mut after = before.clone();
        after.reverse();
        after[0].description = "new".to_owned();
        after.push(tool("extra", ""));
        let started = std::time::Instant::now();
        let changes = ToolChanges::between(Some(&before), &after[1..]);
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
        assert_eq!(changes.added, ["extra"]);
        assert_eq!(changes.removed, ["tool49999"]);
        assert!(changes.changed.is_empty());
        let changes = ToolChanges::between(Some(&before), &after);
        assert_eq!(changes.changed, ["tool49999"]);
    }

    #[test]
    fn titles_and_annotations_are_not_changes() {
        let before = [tool("read", "Read")];
        let mut after = tool("read", "Read");
        after.title = Some("Read a file".to_owned());
        after.read_only = true;
        assert!(ToolChanges::between(Some(&before), &[after]).is_empty());
    }
}
