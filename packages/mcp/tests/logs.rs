//! What reaches the application's log from MCP traffic, with the filter `copilot-logger`
//! installs. rmcp writes untrusted text into its own events (a failed connection's URL, which
//! may carry a credential in its path, and every server notification with its contents); none
//! of it may reach the log at any level, while copilot-mcp's own `mcp.*` events still do.
//!
//! One test per binary: the subscriber is process-global. A second, unfiltered layer records
//! everything, to prove each case really produced the text the filter keeps out.

// Shared with `tests/mcp.rs`, which uses all of it.
#[allow(dead_code)]
mod server;

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use copilot_core::{BaseUrl, LogLevel, NetworkScope, ToolEndpoint, ToolHost, ToolServerId};
use copilot_mcp::{Limits, McpHost};
use copilot_security::{Purpose, clients};
use serde_json::json;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::server::{Era, Options, default_tools, start, tool};

/// Log lines written to memory.
#[derive(Clone, Default)]
struct Lines(Arc<Mutex<Vec<u8>>>);

impl Lines {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for Lines {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn host() -> McpHost {
    let limits = Limits {
        connect: Duration::from_secs(5),
        list: Duration::from_secs(10),
        ..Limits::default()
    };
    McpHost::with_limits(clients(Purpose::Tools).expect("clients build"), limits)
}

fn endpoint(url: &str) -> ToolEndpoint {
    ToolEndpoint {
        server_id: ToolServerId::generate(),
        url: BaseUrl::restore(url).expect("a URL"),
        headers: None,
        protocol_version: None,
        network: NetworkScope::Private,
    }
}

#[tokio::test]
async fn untrusted_server_text_never_reaches_the_log() {
    let logged = Lines::default();
    let everything = Lines::default();
    let (to_log, to_record) = (logged.clone(), everything.clone());
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(move || to_log.clone())
                // The most a deployment logs: `LOG_LEVEL=debug`.
                .with_filter(copilot_logger::targets(LogLevel::Debug)),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(move || to_record.clone()))
        .init();

    // A server that is down, at a URL with a credential in its path.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let down = endpoint(&format!("http://{address}/s/sk-path-secret-111/mcp"));
    host().list(&down).await.unwrap_err();

    // Servers of both eras that send a log notification during a call.
    for (era, secret) in [
        (Era::Modern, "notification-secret-modern"),
        (Era::Legacy, "notification-secret-legacy"),
    ] {
        let server = start(era, Options::default()).await;
        let mut tools = default_tools();
        tools.push(tool(
            "notify",
            "Sends a notification.",
            json!({"data": {"type": "object"}}),
        ));
        server.set_tools(tools);
        let ready = endpoint(&server.url);
        let session = host().open(vec![ready.clone()]).await.unwrap();
        let output = session
            .call(
                ready.server_id,
                "notify",
                json!({"data": {"row": secret}}),
                Duration::from_secs(10),
            )
            .await
            .unwrap();
        assert!(!output.is_error, "{era:?}");
        session.close().await;
    }
    // rmcp's workers log as they wind down.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let logged = logged.text();
    let everything = everything.text();
    for secret in [
        "sk-path-secret-111",
        "notification-secret-modern",
        "notification-secret-legacy",
    ] {
        assert!(
            everything.contains(secret),
            "rmcp no longer logs {secret}; this case proves nothing now"
        );
        assert!(!logged.contains(secret), "{secret} reached the log");
    }
    assert!(
        logged
            .lines()
            .all(|line| !line.contains("\"target\":\"rmcp")),
        "rmcp's own events never reach the log"
    );
    assert!(
        logged.contains("mcp.http.failed") && logged.contains("mcp.tools.called"),
        "copilot-mcp's own events still do"
    );
}
