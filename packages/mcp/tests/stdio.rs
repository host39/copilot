//! stdio tool servers end to end: the adapter starts a real child process, lists and calls its
//! tools, and ends it.
//!
//! This test binary is its own MCP server. Run with `serve`, it answers MCP on its stdin and
//! stdout (rmcp's stdio server); with `silent`, it never answers; with no argument it starts
//! itself again with `tests` and the environment of an API process (`SECRET_KEY`,
//! `DATABASE_URL`), and that copy runs the tests below one after another (`harness = false`,
//! so no other tests share the process or its children). A failing test panics, which fails the
//! run.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use copilot_core::{
    DomainError, DomainResult, NetworkScope, Secret, SpawnCheck, SpawnGate, StdioEnv, StdioLaunch,
    TOOL_TIMEOUT, Target, ToolContent, ToolEndpoint, ToolHost, ToolOutput, ToolServerId,
    ToolSession,
};
use copilot_mcp::{Limits, McpHost, TOOL_SERVER_START};
use copilot_security::{Purpose, clients};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler, ServiceExt};
use serde_json::{Value, json};

/// What the API's own environment holds and no child may see.
const SECRET: &str = "api-secret-key-that-must-not-reach-children";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") => serve(),
        Some("silent") => silent(),
        Some("tests") => tests(),
        // `make test-db` runs the ignored tests; there are none here.
        Some("--ignored" | "--list") => {}
        _ => parent(),
    }
}

/// Runs the tests in a copy of this binary whose environment is like the API's.
fn parent() {
    let status = std::process::Command::new(std::env::current_exe().expect("own path"))
        .arg("tests")
        .env("SECRET_KEY", SECRET)
        .env("DATABASE_URL", "postgres://copilot:db-password@db/copilot")
        .status()
        .expect("the tests run");
    std::process::exit(status.code().unwrap_or(1));
}

// The server side ---------------------------------------------------------------------------

/// Answers MCP on stdin and stdout until stdin closes. Writes `started` (its process id) into
/// its working directory first, so the tests can tell whether it ever ran.
fn serve() {
    std::fs::write("started", std::process::id().to_string()).expect("the directory is writable");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async {
        let service = Program
            .serve(rmcp::transport::stdio())
            .await
            .expect("the handshake");
        let _ = service.waiting().await;
    });
}

/// Never answers: writes its process id into `pid` in its working directory and waits forever.
fn silent() {
    std::fs::write("pid", std::process::id().to_string()).expect("the directory is writable");
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

#[derive(Clone)]
struct Program;

fn tool(name: &'static str, description: &'static str) -> Tool {
    let Value::Object(schema) = json!({"type": "object"}) else {
        unreachable!("an object literal");
    };
    Tool::new(name, description, schema)
}

fn text(content: String) -> CallToolResponse {
    CallToolResult::success(vec![ContentBlock::text(content)]).into()
}

impl ServerHandler for Program {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(vec![
            tool("echo", "Echoes text."),
            tool("env", "Names every environment variable it sees."),
            tool("cwd", "Names its working directory."),
            tool("pid", "Its process id."),
            tool("child", "Starts a child that never ends."),
            tool("sleep", "Waits."),
            tool("shout", "Writes lines to stderr."),
        ]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let argument = |name: &str| {
            request
                .arguments
                .as_ref()
                .and_then(|arguments| arguments.get(name))
                .cloned()
                .unwrap_or(Value::Null)
        };
        Ok(match request.name.as_ref() {
            "echo" => text(format!("echo: {}", argument("text").as_str().unwrap_or(""))),
            "env" => {
                let mut names: Vec<String> = std::env::vars().map(|(name, _)| name).collect();
                names.sort();
                let visible = std::env::var("VISIBLE").unwrap_or_default();
                text(format!("{}={visible}", names.join(",")))
            }
            "cwd" => text(
                std::env::current_dir()
                    .expect("a directory")
                    .display()
                    .to_string(),
            ),
            "pid" => text(std::process::id().to_string()),
            "child" => {
                // Never waited for on purpose: the test checks that the group kill ends it.
                #[allow(clippy::zombie_processes)]
                let child = std::process::Command::new(std::env::current_exe().expect("own path"))
                    .arg("silent")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .expect("the child starts");
                text(child.id().to_string())
            }
            "sleep" => {
                let ms = argument("ms").as_u64().unwrap_or(0);
                tokio::time::sleep(Duration::from_millis(ms)).await;
                text("done".to_owned())
            }
            "shout" => {
                let lines = argument("lines").as_u64().unwrap_or(0);
                let mut stderr = std::io::stderr().lock();
                for line in 0..lines {
                    writeln!(stderr, "line {line}\x1b[31m red").expect("stderr is open");
                }
                text("shouted".to_owned())
            }
            other => {
                return Err(ErrorData::invalid_params(
                    format!("Unknown tool: {other}"),
                    None,
                ));
            }
        })
    }
}

// The client side ---------------------------------------------------------------------------

/// The core gate as a test sees it: allows or refuses, and counts.
#[derive(Default)]
struct Gate {
    refuse: AtomicBool,
    checks: AtomicUsize,
}

#[async_trait]
impl SpawnCheck for Gate {
    async fn allow(&self) -> DomainResult<()> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        if self.refuse.load(Ordering::SeqCst) {
            Err(DomainError::forbidden(
                "Only administrators can add, change, or run tool servers that run a program on this server.",
            ))
        } else {
            Ok(())
        }
    }
}

impl Gate {
    fn checks(&self) -> usize {
        self.checks.load(Ordering::SeqCst)
    }
}

struct Fixture {
    host: McpHost,
    base: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "copilot-stdio-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("after 1970")
                .as_nanos()
        ));
        let limits = Limits {
            connect: Duration::from_secs(5),
            list: Duration::from_secs(10),
            stdio_start: Duration::from_secs(10),
            close: Duration::from_secs(5),
            ..Limits::default()
        };
        let host = McpHost::with_limits(clients(Purpose::Tools).expect("clients build"), limits)
            .with_stdio(base.clone());
        Self { host, base }
    }

    /// A host whose processes must finish the handshake within `start`.
    fn impatient(&self, start: Duration) -> McpHost {
        let limits = Limits {
            stdio_start: start,
            list: Duration::from_secs(5),
            ..Limits::default()
        };
        McpHost::with_limits(clients(Purpose::Tools).expect("clients build"), limits)
            .with_stdio(self.base.clone())
    }

    fn dir(&self, cwd: &str) -> PathBuf {
        self.base.join(cwd)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn endpoint(args: &[&str], cwd: &str, gate: &Arc<Gate>) -> ToolEndpoint {
    let env = BTreeMap::from([("VISIBLE".to_owned(), "yes".to_owned())]);
    ToolEndpoint {
        server_id: ToolServerId::generate(),
        target: Target::Stdio(StdioLaunch {
            command: std::env::current_exe()
                .expect("own path")
                .display()
                .to_string(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            env: Some(Secret::new(StdioEnv::new(&env).expect("valid").to_json())),
            cwd: cwd.to_owned(),
            gate: SpawnGate(gate.clone()),
        }),
        headers: None,
        tokens: None,
        protocol_version: None,
        network: NetworkScope::Public,
    }
}

const CALL: Duration = Duration::from_secs(10);

fn output_text(output: &ToolOutput) -> String {
    output
        .content
        .iter()
        .map(|content| match content {
            ToolContent::Text(text) => text.clone(),
            other => panic!("not text: {other:?}"),
        })
        .collect()
}

async fn call(
    session: &dyn ToolSession,
    endpoint: &ToolEndpoint,
    name: &str,
    arguments: Value,
) -> String {
    let output = session
        .call(endpoint.server_id, name, arguments, CALL)
        .await
        .unwrap_or_else(|error| panic!("{name}: {error}"));
    output_text(&output)
}

/// Whether the process is gone (or only a zombie waiting for its parent to read its status).
fn gone(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Err(_) => true,
        Ok(stat) => stat
            .rsplit(')')
            .next()
            .is_some_and(|rest| rest.trim_start().starts_with('Z')),
    }
}

async fn wait_gone(pid: u32, what: &str) {
    let started = Instant::now();
    while !gone(pid) {
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "{what} (pid {pid}) is still running"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn read_pid(path: &Path) -> u32 {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .trim()
        .parse()
        .expect("a process id")
}

// The tests ---------------------------------------------------------------------------------

fn tests() {
    let logs = Logs::install();
    let runtime = tokio::runtime::Runtime::new().expect("a runtime");
    runtime.block_on(async {
        assert_eq!(
            std::env::var("SECRET_KEY").as_deref(),
            Ok(SECRET),
            "set up like the API"
        );
        lists_and_calls_tools_in_the_server_directory().await;
        the_process_gets_only_its_listed_environment().await;
        the_gate_decides_before_every_start().await;
        a_refused_start_fails_every_call_of_the_run().await;
        no_directory_means_no_stdio().await;
        a_missing_program_is_reported().await;
        a_program_that_never_answers_is_killed().await;
        the_process_and_its_children_end_with_the_session().await;
        dropping_the_session_kills_the_process().await;
        a_call_that_runs_too_long_times_out().await;
        stderr_goes_to_the_log_bounded(&logs).await;
    });
}

async fn lists_and_calls_tools_in_the_server_directory() {
    let f = Fixture::new();
    let gate = Arc::new(Gate::default());
    let endpoint = endpoint(&["serve"], "team/files", &gate);
    let listing = f.host.list(&endpoint).await.expect("listed");
    let names: Vec<&str> = listing
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["echo", "env", "cwd", "pid", "child", "sleep", "shout"]
    );
    assert!(!listing.protocol_version.is_empty());
    let started = read_pid(&f.dir("team/files").join("started"));
    wait_gone(started, "the listing's process").await;

    let session = f
        .host
        .open(vec![endpoint.clone()])
        .await
        .expect("a session");
    assert_eq!(
        call(&*session, &endpoint, "echo", json!({"text": "hi"})).await,
        "echo: hi"
    );
    let cwd = call(&*session, &endpoint, "cwd", json!({})).await;
    assert_eq!(
        PathBuf::from(cwd),
        f.dir("team/files").canonicalize().expect("made")
    );
    // One process per server per run: the second call used the same one.
    let pid = call(&*session, &endpoint, "pid", json!({})).await;
    assert_eq!(
        call(&*session, &endpoint, "pid", json!({})).await,
        pid,
        "one process for the run"
    );
    session.close().await;
    wait_gone(pid.parse().expect("a pid"), "the run's process").await;
    assert_eq!(gate.checks(), 2, "one check per start");
}

async fn the_process_gets_only_its_listed_environment() {
    let f = Fixture::new();
    let gate = Arc::new(Gate::default());
    let endpoint = endpoint(&["serve"], "env", &gate);
    let session = f
        .host
        .open(vec![endpoint.clone()])
        .await
        .expect("a session");
    let env = call(&*session, &endpoint, "env", json!({})).await;
    assert_eq!(
        env, "VISIBLE=yes",
        "env_clear, then only the listed variables"
    );
    assert!(!env.contains("SECRET_KEY") && !env.contains(SECRET));
    assert!(!env.contains("DATABASE_URL") && !env.contains("PATH"));
    session.close().await;
}

async fn the_gate_decides_before_every_start() {
    let f = Fixture::new();
    let gate = Arc::new(Gate::default());
    let endpoint = endpoint(&["serve"], "gated", &gate);
    f.host.list(&endpoint).await.expect("allowed");
    f.host.list(&endpoint).await.expect("allowed again");
    assert_eq!(gate.checks(), 2);
    std::fs::remove_file(f.dir("gated").join("started")).expect("it ran");

    // Demoted, or stdio turned off: the next start is refused with the gate's own error, and
    // nothing runs.
    gate.refuse.store(true, Ordering::SeqCst);
    let error = f.host.list(&endpoint).await.unwrap_err();
    assert_eq!(error.code(), "FORBIDDEN");
    assert!(error.to_string().contains("Only administrators"), "{error}");
    assert_eq!(gate.checks(), 3);
    assert!(
        !f.dir("gated").join("started").exists(),
        "the program never started"
    );
}

async fn a_refused_start_fails_every_call_of_the_run() {
    let f = Fixture::new();
    let gate = Arc::new(Gate::default());
    gate.refuse.store(true, Ordering::SeqCst);
    let endpoint = endpoint(&["serve"], "refused", &gate);
    let session = f
        .host
        .open(vec![endpoint.clone()])
        .await
        .expect("a session");
    for _ in 0..2 {
        let error = session
            .call(endpoint.server_id, "echo", json!({}), CALL)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Only administrators"), "{error}");
    }
    assert_eq!(
        gate.checks(),
        1,
        "a failed server is not tried again in the run"
    );
    assert!(!f.dir("refused").exists(), "not even its directory");
    session.close().await;
}

async fn no_directory_means_no_stdio() {
    let host = McpHost::new(clients(Purpose::Tools).expect("clients build"));
    let gate = Arc::new(Gate::default());
    let error = host
        .list(&endpoint(&["serve"], "none", &gate))
        .await
        .unwrap_err();
    assert_eq!(error.code(), TOOL_SERVER_START);
    assert!(error.to_string().contains("MCP_STDIO_DIR"), "{error}");
}

async fn a_missing_program_is_reported() {
    let f = Fixture::new();
    let gate = Arc::new(Gate::default());
    let mut endpoint = endpoint(&[], "missing", &gate);
    if let Target::Stdio(launch) = &mut endpoint.target {
        launch.command = "/nonexistent/copilot-test-program".to_owned();
    }
    let error = f.host.list(&endpoint).await.unwrap_err();
    assert_eq!(error.code(), TOOL_SERVER_START);
    assert!(error.to_string().contains("not found"), "{error}");
}

async fn a_program_that_never_answers_is_killed() {
    let f = Fixture::new();
    let gate = Arc::new(Gate::default());
    let host = f.impatient(Duration::from_secs(1));
    let endpoint = endpoint(&["silent"], "silent", &gate);
    let started = Instant::now();
    let error = host.list(&endpoint).await.unwrap_err();
    assert_eq!(error.code(), "TOOL_SERVER_TIMEOUT", "{error}");
    assert!(started.elapsed() < Duration::from_secs(5));
    let pid = read_pid(&f.dir("silent").join("pid"));
    wait_gone(pid, "the silent program").await;
}

async fn the_process_and_its_children_end_with_the_session() {
    let f = Fixture::new();
    let gate = Arc::new(Gate::default());
    let endpoint = endpoint(&["serve"], "family", &gate);
    let session = f
        .host
        .open(vec![endpoint.clone()])
        .await
        .expect("a session");
    let pid: u32 = call(&*session, &endpoint, "pid", json!({}))
        .await
        .parse()
        .expect("a pid");
    let child: u32 = call(&*session, &endpoint, "child", json!({}))
        .await
        .parse()
        .expect("a pid");
    assert!(!gone(pid) && !gone(child));
    session.close().await;
    wait_gone(pid, "the server").await;
    wait_gone(child, "the server's child").await;
}

async fn dropping_the_session_kills_the_process() {
    let f = Fixture::new();
    let gate = Arc::new(Gate::default());
    let endpoint = endpoint(&["serve"], "dropped", &gate);
    let session = f
        .host
        .open(vec![endpoint.clone()])
        .await
        .expect("a session");
    let pid: u32 = call(&*session, &endpoint, "pid", json!({}))
        .await
        .parse()
        .expect("a pid");
    drop(session);
    wait_gone(pid, "the dropped session's process").await;
}

async fn a_call_that_runs_too_long_times_out() {
    let f = Fixture::new();
    let gate = Arc::new(Gate::default());
    let endpoint = endpoint(&["serve"], "slow", &gate);
    let session = f
        .host
        .open(vec![endpoint.clone()])
        .await
        .expect("a session");
    call(&*session, &endpoint, "echo", json!({})).await;
    let error = session
        .call(
            endpoint.server_id,
            "sleep",
            json!({"ms": 10_000}),
            Duration::from_millis(500),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), TOOL_TIMEOUT);
    session.close().await;
}

async fn stderr_goes_to_the_log_bounded(logs: &Logs) {
    let f = Fixture::new();
    let gate = Arc::new(Gate::default());
    let endpoint = endpoint(&["serve"], "loud", &gate);
    let session = f
        .host
        .open(vec![endpoint.clone()])
        .await
        .expect("a session");
    call(&*session, &endpoint, "shout", json!({"lines": 250})).await;
    session.close().await;
    let id = endpoint.server_id.to_string();
    let started = Instant::now();
    loop {
        let events = logs.events();
        let lines: Vec<&Value> = events
            .iter()
            .filter(|event| {
                event["fields"]["message"] == "mcp.stdio.stderr"
                    && event["fields"]["server_id"] == id
            })
            .collect();
        let dropped = events.iter().find(|event| {
            event["fields"]["message"] == "mcp.stdio.stderr-dropped"
                && event["fields"]["server_id"] == id
        });
        if let Some(dropped) = dropped {
            assert_eq!(lines.len(), 200, "at most STDIO_STDERR_MAX_LINES lines");
            assert_eq!(dropped["fields"]["dropped"], 50);
            assert_eq!(lines[0]["level"], "DEBUG");
            assert_eq!(
                lines[0]["fields"]["line"], "line 0 [31m red",
                "no escape characters"
            );
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "no mcp.stdio.stderr-dropped event"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Every log event of the run, as JSON lines in memory.
#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    fn install() -> Self {
        let logs = Self::default();
        let writer = logs.clone();
        tracing_subscriber::fmt()
            .json()
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || writer.clone())
            .init();
        logs
    }

    fn events(&self) -> Vec<Value> {
        let bytes = self.0.lock().expect("log lock").clone();
        String::from_utf8_lossy(&bytes)
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }
}

impl Write for Logs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("log lock").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
