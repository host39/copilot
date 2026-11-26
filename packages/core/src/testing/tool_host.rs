//! A scripted `ToolHost`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::ports::tool_host::{
    RemoteTool, ToolContent, ToolEndpoint, ToolHost, ToolListing, ToolOutput, ToolSession,
};
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::ToolServerId;

/// What the next tool call does.
#[derive(Debug)]
pub enum CallStep {
    Answer(DomainResult<ToolOutput>),
    /// The call never answers (to test cancellation).
    Stall,
}

/// A tool call a session received.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordedCall {
    pub server_id: ToolServerId,
    pub name: String,
    pub arguments: Value,
    pub timeout: Duration,
}

/// Answers each `list` call with the next scripted result, in order, and records every endpoint
/// it was called with. An unscripted call fails with an upstream-style business rule error.
/// Sessions answer calls from one shared script, in order.
#[derive(Debug, Default)]
pub struct ScriptedToolHost {
    listings: Mutex<VecDeque<DomainResult<ToolListing>>>,
    endpoints: Mutex<Vec<ToolEndpoint>>,
    sessions: Arc<Sessions>,
}

#[derive(Debug, Default)]
struct Sessions {
    steps: Mutex<VecDeque<CallStep>>,
    calls: Mutex<Vec<RecordedCall>>,
    opened: Mutex<Vec<Vec<ToolEndpoint>>>,
    closed: Mutex<usize>,
}

impl ScriptedToolHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// The next `list` call returns these tools (revision `2026-07-28`, no `ttlMs`).
    pub fn push_tools(&self, tools: Vec<RemoteTool>) {
        self.push_listing(ToolListing {
            tools,
            protocol_version: "2026-07-28".to_owned(),
            ttl: None,
        });
    }

    pub fn push_listing(&self, listing: ToolListing) {
        self.listings
            .lock()
            .expect("tool host lock")
            .push_back(Ok(listing));
    }

    /// The next `list` call fails with this error.
    pub fn push_error(&self, error: DomainError) {
        self.listings
            .lock()
            .expect("tool host lock")
            .push_back(Err(error));
    }

    /// The next tool call answers with this text.
    pub fn push_text(&self, text: &str) {
        self.push_output(ToolOutput {
            content: vec![ToolContent::Text(text.to_owned())],
            is_error: false,
        });
    }

    /// The next tool call answers with this output.
    pub fn push_output(&self, output: ToolOutput) {
        self.push_step(CallStep::Answer(Ok(output)));
    }

    /// The next tool call fails with this error.
    pub fn push_call_error(&self, error: DomainError) {
        self.push_step(CallStep::Answer(Err(error)));
    }

    pub fn push_step(&self, step: CallStep) {
        self.sessions
            .steps
            .lock()
            .expect("tool host lock")
            .push_back(step);
    }

    /// The endpoints of every `list` call so far, oldest first.
    pub fn endpoints(&self) -> Vec<ToolEndpoint> {
        self.endpoints.lock().expect("tool host lock").clone()
    }

    /// Every tool call any session received, oldest first.
    pub fn calls(&self) -> Vec<RecordedCall> {
        self.sessions.calls.lock().expect("tool host lock").clone()
    }

    /// The endpoints of every opened session, oldest first.
    pub fn opened(&self) -> Vec<Vec<ToolEndpoint>> {
        self.sessions.opened.lock().expect("tool host lock").clone()
    }

    /// How many sessions were closed.
    pub fn closed(&self) -> usize {
        *self.sessions.closed.lock().expect("tool host lock")
    }
}

#[async_trait]
impl ToolHost for ScriptedToolHost {
    async fn list(&self, endpoint: &ToolEndpoint) -> DomainResult<ToolListing> {
        self.endpoints
            .lock()
            .expect("tool host lock")
            .push(endpoint.clone());
        self.listings
            .lock()
            .expect("tool host lock")
            .pop_front()
            .unwrap_or_else(|| Err(unreachable()))
    }

    async fn open(&self, endpoints: Vec<ToolEndpoint>) -> DomainResult<Box<dyn ToolSession>> {
        self.sessions
            .opened
            .lock()
            .expect("tool host lock")
            .push(endpoints);
        Ok(Box::new(ScriptedSession(self.sessions.clone())))
    }
}

struct ScriptedSession(Arc<Sessions>);

#[async_trait]
impl ToolSession for ScriptedSession {
    async fn call(
        &self,
        server_id: ToolServerId,
        name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> DomainResult<ToolOutput> {
        self.0
            .calls
            .lock()
            .expect("tool host lock")
            .push(RecordedCall {
                server_id,
                name: name.to_owned(),
                arguments,
                timeout,
            });
        let step = self.0.steps.lock().expect("tool host lock").pop_front();
        match step {
            Some(CallStep::Answer(answer)) => answer,
            Some(CallStep::Stall) => std::future::pending().await,
            None => Err(unreachable()),
        }
    }

    async fn close(self: Box<Self>) {
        *self.0.closed.lock().expect("tool host lock") += 1;
    }
}

fn unreachable() -> DomainError {
    DomainError::business_rule(
        "The tool server could not be reached.",
        "TOOL_SERVER_UNREACHABLE",
    )
}

/// A listed tool with an empty object schema and no annotations.
pub fn remote_tool(name: &str) -> RemoteTool {
    RemoteTool {
        name: name.to_owned(),
        title: None,
        description: format!("The {name} tool."),
        input_schema: json!({"type": "object"}),
        read_only: false,
    }
}
