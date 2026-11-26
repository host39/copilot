//! The approval policy (`docs/design/copilot.md`, Tool Execution and Approvals): whether a tool
//! call runs, asks the user, is refused, or fails. First match wins.

use crate::domains::chat::{ApprovedTool, RunMode, ToolCallPart, TurnContext};
use crate::domains::schedule::{OnAsk, ScheduleTool};
use crate::domains::tool::{ToolMode, ToolServer};

/// The result of a call whose alias the turn does not offer.
pub const UNKNOWN_TOOL: &str = "Unknown tool.";
/// The result of a call whose arguments are not a JSON object.
pub const INVALID_ARGUMENTS: &str = "Arguments were not valid JSON.";
/// The denial of an `ask` tool in a scheduled run set to "Deny and continue".
pub const CANNOT_ASK: &str = "This tool needs approval, and scheduled runs cannot ask.";
/// The result of a call to a server that was turned off or removed during the turn.
pub const SERVER_GONE: &str = "This tool server is turned off or was removed.";
/// The denial of a call to a tool that was turned off during the turn.
pub const TOOL_OFF: &str = "This tool is turned off.";

/// What happens to a call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Verdict {
    Run,
    /// Wait for the user's decision.
    Ask,
    /// A `denied` result with this reason.
    Deny(String),
    /// An `error` result with this text.
    Fail(String),
}

/// What the policy reads besides the call.
#[derive(Clone, Copy, Debug)]
pub struct Policy<'a> {
    /// The turn's snapshot: only its aliases may run.
    pub context: &'a TurnContext,
    /// The current settings of the snapshot's servers.
    pub servers: &'a [ToolServer],
    /// The chat's "Approve for this chat" tools, by server id and original name.
    pub approved: &'a [ApprovedTool],
    pub mode: RunMode,
}

/// The policy, in order:
///
/// 1. An alias not in the snapshot fails ("Unknown tool.").
/// 2. Arguments that are not a JSON object fail ("Arguments were not valid JSON.").
/// 3. `schedule_prompt` and `cancel_schedule` ask; `list_schedules` runs.
/// 4. A server that was turned off or removed fails.
/// 5. A server whose tool list changed and was not reviewed asks.
/// 6. The tool's mode, else the server's: `auto` runs; `off` (set during the turn) is denied.
/// 7. `ask` runs when the chat approved this server's tool (by server id and original name,
///    never by alias).
/// 8. Otherwise it asks. An unattended run cannot ask: with "Deny and continue" the call is
///    denied, with "Pause until I approve" the run pauses.
pub fn decide(call: &ToolCallPart, policy: &Policy<'_>) -> Verdict {
    let Some(tool) = policy.context.tool(&call.alias) else {
        return Verdict::Fail(UNKNOWN_TOOL.to_owned());
    };
    if call.arguments.is_none() {
        return Verdict::Fail(INVALID_ARGUMENTS.to_owned());
    }
    match tool.server_id {
        None => match ScheduleTool::find(&tool.name) {
            Some(builtin) if builtin.always_asks() => {}
            Some(_) => return Verdict::Run,
            None => return Verdict::Fail(UNKNOWN_TOOL.to_owned()),
        },
        Some(id) => {
            let Some(server) = policy
                .servers
                .iter()
                .find(|server| server.id() == id && server.enabled())
            else {
                return Verdict::Fail(SERVER_GONE.to_owned());
            };
            if !server.review_required() {
                match server.mode(&tool.name) {
                    ToolMode::Auto => return Verdict::Run,
                    ToolMode::Off => return Verdict::Deny(TOOL_OFF.to_owned()),
                    ToolMode::Ask
                        if policy.approved.iter().any(|approved| {
                            approved.server_id == id && approved.name == tool.name
                        }) =>
                    {
                        return Verdict::Run;
                    }
                    ToolMode::Ask => {}
                }
            }
        }
    }
    match policy.mode {
        RunMode::Unattended {
            on_ask: OnAsk::Deny,
        } => Verdict::Deny(CANNOT_ASK.to_owned()),
        RunMode::Interactive
        | RunMode::Unattended {
            on_ask: OnAsk::Pause,
        } => Verdict::Ask,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{DateTime, Utc};
    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::domains::chat::{Approval, ContextHash, SnapshotTool};
    use crate::domains::tool::{ApprovalMode, Slug, ToolAlias};
    use crate::ports::tool_host::{RemoteTool, ToolListing};
    use crate::shared::base_url::BaseUrl;
    use crate::shared::ids::{CallId, ToolServerId, UserId};
    use crate::shared::net::NetworkScope;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_767_225_600, 0).unwrap()
    }

    fn server(approval: ApprovalMode) -> ToolServer {
        let url = BaseUrl::parse("https://mcp.example.com/mcp", NetworkScope::Public).unwrap();
        let server = ToolServer::create(
            UserId::new(Uuid::now_v7()),
            "GitHub",
            Slug::parse("gh").unwrap(),
            url,
            now(),
        )
        .unwrap()
        .with_approval(approval);
        let listing = ToolListing {
            tools: ["search", "delete"]
                .into_iter()
                .map(|name| RemoteTool {
                    name: name.to_owned(),
                    title: None,
                    description: String::new(),
                    input_schema: json!({"type": "object"}),
                    read_only: false,
                })
                .collect(),
            protocol_version: "2026-07-28".to_owned(),
            ttl: None,
        };
        server.with_listing(&listing, false, now())
    }

    fn context(server: &ToolServer) -> TurnContext {
        let mut tools: Vec<SnapshotTool> = ["search", "delete"]
            .into_iter()
            .map(|name| SnapshotTool {
                alias: ToolAlias::new(server.slug(), name),
                server_id: Some(server.id()),
                name: name.to_owned(),
                description: String::new(),
                input_schema: json!({"type": "object"}),
            })
            .collect();
        tools.extend(ScheduleTool::ALL.into_iter().map(|tool| SnapshotTool {
            alias: ToolAlias::builtin(tool.name()),
            server_id: None,
            name: tool.name().to_owned(),
            description: String::new(),
            input_schema: json!({"type": "object"}),
        }));
        TurnContext {
            hash: ContextHash::new([0; 32]),
            system: String::new(),
            tools,
            files: Vec::new(),
            window_start_id: None,
            reasoning: None,
        }
    }

    fn call(alias: &str) -> ToolCallPart {
        ToolCallPart {
            id: CallId::generate(),
            provider_call_id: None,
            server_id: None,
            tool_name: alias.to_owned(),
            alias: ToolAlias::restore(alias.to_owned()),
            arguments_raw: "{}".to_owned(),
            arguments: Some(json!({})),
            complete: true,
            approval: Approval::Auto,
            deny_reason: None,
        }
    }

    fn verdict(call: &ToolCallPart, server: &ToolServer, mode: RunMode) -> Verdict {
        let context = context(server);
        decide(
            call,
            &Policy {
                context: &context,
                servers: std::slice::from_ref(server),
                approved: &[],
                mode,
            },
        )
    }

    const PAUSE: RunMode = RunMode::Unattended {
        on_ask: OnAsk::Pause,
    };
    const DENY: RunMode = RunMode::Unattended {
        on_ask: OnAsk::Deny,
    };

    #[test]
    fn unknown_aliases_and_bad_arguments_fail_first() {
        let auto = server(ApprovalMode::Auto);
        assert_eq!(
            verdict(&call("gh__other"), &auto, RunMode::Interactive),
            Verdict::Fail(UNKNOWN_TOOL.to_owned())
        );
        let bad = ToolCallPart {
            arguments_raw: "{\"q\":".to_owned(),
            arguments: None,
            ..call("gh__search")
        };
        assert_eq!(
            verdict(&bad, &auto, RunMode::Interactive),
            Verdict::Fail(INVALID_ARGUMENTS.to_owned())
        );
    }

    #[test]
    fn schedule_tools_ask_except_the_list() {
        let auto = server(ApprovalMode::Auto);
        for name in ["schedule_prompt", "cancel_schedule"] {
            assert_eq!(
                verdict(&call(name), &auto, RunMode::Interactive),
                Verdict::Ask
            );
        }
        assert_eq!(
            verdict(&call("list_schedules"), &auto, RunMode::Interactive),
            Verdict::Run
        );
    }

    #[test]
    fn modes_decide_server_tools() {
        let auto = server(ApprovalMode::Auto);
        assert_eq!(
            verdict(&call("gh__search"), &auto, RunMode::Interactive),
            Verdict::Run
        );
        let ask = server(ApprovalMode::Ask);
        assert_eq!(
            verdict(&call("gh__search"), &ask, RunMode::Interactive),
            Verdict::Ask
        );
        let per_tool = ask.clone().with_tool_modes(BTreeMap::from([
            ("search".to_owned(), ToolMode::Auto),
            ("delete".to_owned(), ToolMode::Off),
        ]));
        assert_eq!(
            verdict(&call("gh__search"), &per_tool, RunMode::Interactive),
            Verdict::Run
        );
        assert_eq!(
            verdict(&call("gh__delete"), &per_tool, RunMode::Interactive),
            Verdict::Deny(TOOL_OFF.to_owned())
        );
    }

    #[test]
    fn a_changed_tool_list_asks_even_for_auto_tools() {
        let auto = server(ApprovalMode::Auto);
        let listing = ToolListing {
            tools: Vec::new(),
            protocol_version: "2026-07-28".to_owned(),
            ttl: None,
        };
        let changed = auto.with_listing(&listing, true, now());
        assert_eq!(
            verdict(&call("gh__search"), &changed, RunMode::Interactive),
            Verdict::Ask
        );
        assert_eq!(
            verdict(&call("gh__search"), &changed, DENY),
            Verdict::Deny(CANNOT_ASK.to_owned())
        );
    }

    #[test]
    fn chat_approvals_run_ask_tools() {
        let ask = server(ApprovalMode::Ask);
        let context = context(&ask);
        let approved = [ApprovedTool {
            server_id: ask.id(),
            name: "search".to_owned(),
        }];
        let policy = Policy {
            context: &context,
            servers: std::slice::from_ref(&ask),
            approved: &approved,
            mode: RunMode::Interactive,
        };
        assert_eq!(decide(&call("gh__search"), &policy), Verdict::Run);
        assert_eq!(decide(&call("gh__delete"), &policy), Verdict::Ask);
        let elsewhere = [ApprovedTool {
            server_id: ToolServerId::generate(),
            name: "search".to_owned(),
        }];
        let other = Policy {
            approved: &elsewhere,
            ..policy
        };
        assert_eq!(
            decide(&call("gh__search"), &other),
            Verdict::Ask,
            "an approval of another server's tool with the same name"
        );
        assert_eq!(
            decide(&call("schedule_prompt"), &policy),
            Verdict::Ask,
            "schedule tools always ask"
        );
    }

    #[test]
    fn unattended_runs_deny_or_pause() {
        let ask = server(ApprovalMode::Ask);
        assert_eq!(
            verdict(&call("gh__search"), &ask, DENY),
            Verdict::Deny(CANNOT_ASK.to_owned())
        );
        assert_eq!(verdict(&call("gh__search"), &ask, PAUSE), Verdict::Ask);
        let auto = server(ApprovalMode::Auto);
        assert_eq!(verdict(&call("gh__search"), &auto, DENY), Verdict::Run);
    }

    #[test]
    fn a_server_turned_off_during_the_turn_fails_its_calls() {
        let off = server(ApprovalMode::Auto).with_enabled(false);
        assert_eq!(
            verdict(&call("gh__search"), &off, RunMode::Interactive),
            Verdict::Fail(SERVER_GONE.to_owned())
        );
        let context = context(&off);
        let policy = Policy {
            context: &context,
            servers: &[],
            approved: &[],
            mode: RunMode::Interactive,
        };
        assert_eq!(
            decide(&call("gh__search"), &policy),
            Verdict::Fail(SERVER_GONE.to_owned())
        );
    }
}
