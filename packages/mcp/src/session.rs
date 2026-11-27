//! [`McpSession`], the tool calls of one run: one connection per server, opened on its first
//! call and kept until the run closes the session.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use copilot_core::{
    DomainError, DomainResult, Secret, TOOL_TIMEOUT, ToolEndpoint, ToolOutput, ToolServerId,
    ToolSession,
};
use futures_util::future::join_all;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::connection::{CallError, Connection};
use crate::errors::{Credentials, Failure, TOOL_SERVER_FAILED};
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
        let Slot::Open(connection) = slot else {
            let Slot::Failed { message, code } = slot else {
                unreachable!("a slot is open or failed");
            };
            return Err(DomainError::upstream(message.clone(), code.clone()));
        };
        match connection.call(name, arguments).await {
            Ok(output) => Ok(output),
            Err(CallError::Failed(error)) => Err(error),
            Err(CallError::Unauthorized) => {
                // The access token was refused mid-run: refresh it once, reconnect, call again.
                // The server refused the request before running the tool, so calling again is
                // safe.
                tracing::info!(server_id = %server_id, "mcp.oauth.token-rejected");
                let rejected = connection.token().cloned();
                let fresh = self.reconnect(endpoint, rejected).await?;
                let stale = std::mem::replace(connection, fresh);
                stale.close(self.shared.limits.close).await;
                match connection.call(name, arguments).await {
                    Ok(output) => Ok(output),
                    Err(CallError::Failed(error)) => Err(error),
                    Err(CallError::Unauthorized) => {
                        Err(Failure::Auth.report(server_id, Credentials::SignIn))
                    }
                }
            }
        }
    }

    /// A new connection after the server refused `rejected`, with a refreshed access token.
    async fn reconnect(
        &self,
        endpoint: &ToolEndpoint,
        rejected: Option<Secret>,
    ) -> DomainResult<Connection> {
        let (Some(tokens), Some(rejected)) = (&endpoint.tokens, rejected) else {
            return Err(Failure::Auth.report(endpoint.server_id, Credentials::of(endpoint)));
        };
        let fresh = tokens.0.rejected(&rejected).await?;
        Connection::with_token(&self.shared, endpoint, fresh).await
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
