//! A scripted `ToolHost`.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::json;

use crate::ports::tool_host::{RemoteTool, ToolEndpoint, ToolHost, ToolListing};
use crate::shared::errors::{DomainError, DomainResult};

/// Answers each `list` call with the next scripted result, in order, and records every endpoint
/// it was called with. An unscripted call fails with an upstream-style business rule error.
#[derive(Debug, Default)]
pub struct ScriptedToolHost {
    listings: Mutex<VecDeque<DomainResult<ToolListing>>>,
    endpoints: Mutex<Vec<ToolEndpoint>>,
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

    /// The endpoints of every call so far, oldest first.
    pub fn endpoints(&self) -> Vec<ToolEndpoint> {
        self.endpoints.lock().expect("tool host lock").clone()
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
            .unwrap_or_else(|| {
                Err(DomainError::business_rule(
                    "The tool server could not be reached.",
                    "TOOL_SERVER_UNREACHABLE",
                ))
            })
    }
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
