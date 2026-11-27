//! [`McpSession`], the tool calls of one run: one connection per server, opened on its first
//! call and kept until the run closes the session.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use copilot_core::{
    DomainError, DomainResult, TOOL_TIMEOUT, ToolEndpoint, ToolOutput, ToolServerId, ToolSession,
};
use futures_util::future::join_all;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::connection::Connection;
use crate::errors::TOOL_SERVER_FAILED;
use crate::host::Shared;

/// A server's connection in this session.
enum Slot {
    Open(Connection),
    /// The connection failed; later calls fail the same way without trying again.
    Failed {
        message: String,
        code: String,
    },
}

pub(crate) struct McpSession {
    shared: Arc<Shared>,
    endpoints: HashMap<ToolServerId, ToolEndpoint>,
    /// Calls run one at a time (the engine calls in order), so one lock is enough.
    slots: Mutex<HashMap<ToolServerId, Slot>>,
}

impl McpSession {
    pub(crate) fn new(shared: Arc<Shared>, endpoints: Vec<ToolEndpoint>) -> Self {
        Self {
            shared,
            endpoints: endpoints
                .into_iter()
                .map(|endpoint| (endpoint.server_id, endpoint))
                .collect(),
            slots: Mutex::new(HashMap::new()),
        }
    }

    async fn run(
        &self,
        server_id: ToolServerId,
        name: &str,
        arguments: &Value,
    ) -> DomainResult<ToolOutput> {
        let Some(endpoint) = self.endpoints.get(&server_id) else {
            return Err(DomainError::upstream(
                "This tool server is not part of this run.",
                TOOL_SERVER_FAILED,
            ));
        };
        let mut slots = self.slots.lock().await;
        // A connection whose transport ended (the server went away) is opened again.
        if matches!(slots.get(&server_id), Some(Slot::Open(connection)) if connection.is_closed()) {
            slots.remove(&server_id);
        }
        let slot = match slots.entry(server_id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let slot = match Connection::open(&self.shared, endpoint).await {
                    Ok(connection) => Slot::Open(connection),
                    Err(error) => Slot::Failed {
                        message: error.to_string(),
                        code: error.code().to_owned(),
                    },
                };
                entry.insert(slot)
            }
        };
        match slot {
            Slot::Open(connection) => connection.call(name, arguments).await,
            Slot::Failed { message, code } => {
                Err(DomainError::upstream(message.clone(), code.clone()))
            }
        }
    }
}

#[async_trait]
impl ToolSession for McpSession {
    /// `timeout` covers the whole call: connecting on the first call, the call, and an
    /// unknown-tool refresh. Past it the call fails with `TOOL_TIMEOUT`, and dropping the
    /// request cancels it on the server.
    async fn call(
        &self,
        server_id: ToolServerId,
        name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> DomainResult<ToolOutput> {
        match tokio::time::timeout(timeout, self.run(server_id, name, &arguments)).await {
            Ok(result) => result,
            Err(_) => {
                tracing::warn!(
                    server_id = %server_id,
                    tool = name,
                    timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
                    "mcp.tools.timeout"
                );
                Err(DomainError::upstream(
                    format!(
                        "The tool did not answer within {} seconds.",
                        timeout.as_secs().max(1)
                    ),
                    TOOL_TIMEOUT,
                ))
            }
        }
    }

    async fn close(self: Box<Self>) {
        let limit = self.shared.limits.close;
        let slots = self.slots.into_inner();
        let open = slots.into_values().filter_map(|slot| match slot {
            Slot::Open(connection) => Some(connection.close(limit)),
            Slot::Failed { .. } => None,
        });
        join_all(open).await;
    }
}
