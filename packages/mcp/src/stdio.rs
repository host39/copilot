//! stdio tool servers: a program on the API host that speaks MCP over its stdin and stdout,
//! through rmcp's child-process transport (`docs/design/copilot.md`, stdio Servers).
//!
//! Every start of a process asks core's gate first (`MCP_STDIO_ENABLED`, and the owner is an
//! administrator at that moment), then runs exactly
//! `Command::new(command).args(args).env_clear().envs(listed).current_dir(<MCP_STDIO_DIR>/<cwd>)`:
//! no shell, no inherited environment (the API's `SECRET_KEY` and database password never reach
//! it), in its own directory, as the leader of its own process group so its children end with
//! it. A process that has not finished the MCP handshake within `Limits::stdio_start` is killed.
//! At the end of its run (or its listing) the process is stopped as the MCP specification
//! describes: its stdin closes, then `SIGTERM`, then `SIGKILL`, each after `STOP_GRACE`; whatever
//! is left of its process group is killed. A [`Process`] that is dropped instead (a run that
//! ends without closing its session) kills the whole group at once. Its stderr goes to the log
//! line by line (`mcp.stdio.stderr`, at `debug`: it is the program's own text and may echo tool
//! arguments), bounded per line and per process.

use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use copilot_core::shared::constants::{STDIO_STDERR_LINE_MAX_BYTES, STDIO_STDERR_MAX_LINES};
use copilot_core::{DomainError, DomainResult, StdioEnv, StdioLaunch, ToolServerId};
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop, ProcessGroup};
use rmcp::RoleClient;
use rmcp::transport::async_rw::AsyncRwTransport;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout, Command};

use crate::errors::{TOOL_SERVER_FAILED, TOOL_SERVER_PROTOCOL, TOOL_SERVER_TIMEOUT};

/// The program could not be started: not found, not executable, or its directory could not be
/// made.
pub const TOOL_SERVER_START: &str = "TOOL_SERVER_START";

/// How long a stopping process gets after its stdin closes, and again after `SIGTERM`.
const STOP_GRACE: Duration = Duration::from_secs(2);
/// The signal numbers (the same on Linux and macOS).
const SIGTERM: i32 = 15;
const SIGKILL: i32 = 9;

/// The MCP transport over a process's stdout and stdin.
pub(crate) type Pipes = AsyncRwTransport<RoleClient, ChildStdout, ChildStdin>;

/// Starts the program of `launch` in its directory under `base`, after the gate allowed it.
/// Returns the MCP transport over its stdout and stdin, whose lines are capped at `max_bytes`,
/// and the process, which kills its group when dropped.
pub(crate) async fn spawn(
    base: Option<&Path>,
    server_id: ToolServerId,
    launch: &StdioLaunch,
    max_bytes: usize,
) -> DomainResult<(Pipes, Process)> {
    launch.gate.0.allow().await?;
    let Some(base) = base else {
        tracing::warn!(server_id = %server_id, "mcp.stdio.not-set-up");
        return Err(DomainError::upstream(
            "This Copilot server has no directory for stdio tool servers: its administrator needs to set MCP_STDIO_DIR.",
            TOOL_SERVER_START,
        ));
    };
    let directory = directory(base, &launch.cwd).map_err(|error| {
        tracing::warn!(server_id = %server_id, kind = %error.kind(), "mcp.stdio.directory-failed");
        DomainError::upstream(
            "The stdio server's working directory could not be prepared under MCP_STDIO_DIR.",
            TOOL_SERVER_START,
        )
    })?;
    let env = match &launch.env {
        Some(secret) => StdioEnv::entries(secret)?,
        None => Vec::new(),
    };
    let mut command = Command::new(&launch.command);
    command
        .args(&launch.args)
        .env_clear()
        .envs(env)
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut wrapped = CommandWrap::from(command);
    wrapped.wrap(KillOnDrop).wrap(ProcessGroup::leader());
    let mut child = wrapped.spawn().map_err(|error| {
        tracing::warn!(server_id = %server_id, kind = %error.kind(), "mcp.stdio.start-failed");
        start_failed(&error)
    })?;
    let pid = child.id();
    // From here on a dropped `Process` kills the group.
    let (stdin, stdout, stderr) = (
        child.stdin().take(),
        child.stdout().take(),
        child.stderr().take(),
    );
    let process = Process {
        child: Some(child),
        server_id,
    };
    let (Some(stdin), Some(stdout)) = (stdin, stdout) else {
        return Err(DomainError::upstream(
            "The stdio server's program could not be started.",
            TOOL_SERVER_START,
        ));
    };
    tracing::info!(server_id = %server_id, pid, "mcp.stdio.started");
    if let Some(stderr) = stderr {
        tokio::spawn(forward(server_id, stderr));
    }
    let pipes = AsyncRwTransport::new_client(stdout, stdin).with_max_line_length(max_bytes);
    Ok((pipes, process))
}

/// A running stdio server process, the leader of its own process group.
#[derive(Debug)]
pub(crate) struct Process {
    child: Option<Box<dyn ChildWrapper>>,
    server_id: ToolServerId,
}

impl Process {
    /// Ends the process after its stdin closed (close the connection first): it gets
    /// `STOP_GRACE` to exit, then `SIGTERM` and `STOP_GRACE` more, then `SIGKILL`. The rest of
    /// its group is killed either way, so children it left behind end with it.
    pub(crate) async fn stop(mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let mut how = "exited";
        if tokio::time::timeout(STOP_GRACE, child.wait())
            .await
            .is_err()
        {
            how = "terminated";
            let _ = child.signal(SIGTERM);
            if tokio::time::timeout(STOP_GRACE, child.wait())
                .await
                .is_err()
            {
                how = "killed";
                let _ = Box::into_pin(child.kill()).await;
            }
        }
        // The leader is gone; its group may not be. Its id stays the group's while any member
        // lives, so this reaches only what the program started.
        let _ = child.signal(SIGKILL);
        tracing::info!(server_id = %self.server_id, how, "mcp.stdio.stopped");
    }
}

impl Drop for Process {
    /// Kills the whole group at once; tokio reaps the leader (`kill_on_drop`).
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            tracing::info!(server_id = %self.server_id, how = "dropped", "mcp.stdio.stopped");
        }
    }
}

/// `<base>/<cwd>`, made (owner-only) when missing, and still inside `base` once symbolic links
/// are resolved. Core already checked that `cwd` is a relative path of plain names.
fn directory(base: &Path, cwd: &str) -> io::Result<PathBuf> {
    let relative = Path::new(cwd);
    let plain = !cwd.is_empty()
        && relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if !plain {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a plain relative path",
        ));
    }
    let directory = base.join(relative);
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)?;
    let root = base.canonicalize()?;
    let resolved = directory.canonicalize()?;
    if !resolved.starts_with(&root) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "outside the base directory",
        ));
    }
    Ok(resolved)
}

fn start_failed(error: &io::Error) -> DomainError {
    let message = match error.kind() {
        io::ErrorKind::NotFound => {
            "The stdio server's program was not found on the Copilot server. Check its path."
        }
        io::ErrorKind::PermissionDenied => {
            "The stdio server's program may not be run by the Copilot server (permission denied)."
        }
        _ => "The stdio server's program could not be started.",
    };
    DomainError::upstream(message, TOOL_SERVER_START)
}

/// How a started process failed the MCP handshake.
pub(crate) fn handshake_failed(timeout: bool, protocol: bool, version: bool) -> DomainError {
    if timeout {
        DomainError::upstream(
            "The stdio server's program did not finish the MCP handshake in time, so it was stopped.",
            TOOL_SERVER_TIMEOUT,
        )
    } else if version {
        DomainError::upstream(
            "The stdio server's program supports no MCP version that Copilot speaks.",
            TOOL_SERVER_PROTOCOL,
        )
    } else if protocol {
        DomainError::upstream(
            "The stdio server's program did not answer with MCP on its standard output. Check its arguments.",
            TOOL_SERVER_PROTOCOL,
        )
    } else {
        DomainError::upstream(
            "The stdio server's program ended before the MCP handshake finished. Its error output is in the server log (mcp.stdio.stderr, at debug level).",
            TOOL_SERVER_FAILED,
        )
    }
}

/// Reads the process's stderr to its end, so the program never blocks on a full pipe, and logs
/// each line: cut to `STDIO_STDERR_LINE_MAX_BYTES`, control characters as spaces, at most
/// `STDIO_STDERR_MAX_LINES` lines; the rest only counts.
async fn forward(server_id: ToolServerId, stderr: ChildStderr) {
    let mut reader = BufReader::new(stderr);
    let mut lines = Lines::default();
    loop {
        let chunk = match reader.fill_buf().await {
            Ok([]) | Err(_) => break,
            Ok(chunk) => chunk,
        };
        let used = lines.take(chunk, |line| {
            tracing::debug!(server_id = %server_id, line, "mcp.stdio.stderr");
        });
        reader.consume(used);
    }
    lines.finish(|line| tracing::debug!(server_id = %server_id, line, "mcp.stdio.stderr"));
    if lines.dropped > 0 {
        tracing::debug!(server_id = %server_id, dropped = lines.dropped, "mcp.stdio.stderr-dropped");
    }
}

/// Bounded line splitting for [`forward`].
#[derive(Default)]
struct Lines {
    current: Vec<u8>,
    logged: usize,
    dropped: usize,
}

impl Lines {
    /// Takes `chunk` up to and including its first newline (all of it without one), keeping at
    /// most `STDIO_STDERR_LINE_MAX_BYTES` of the line; a finished line goes to `emit`. Returns
    /// how many bytes it took.
    fn take(&mut self, chunk: &[u8], emit: impl FnOnce(&str)) -> usize {
        let (part, newline) = match chunk.iter().position(|byte| *byte == b'\n') {
            Some(index) => (&chunk[..index], true),
            None => (chunk, false),
        };
        let room = STDIO_STDERR_LINE_MAX_BYTES.saturating_sub(self.current.len());
        self.current
            .extend_from_slice(&part[..part.len().min(room)]);
        if newline {
            self.finish(emit);
        }
        part.len() + usize::from(newline)
    }

    /// The line so far, if any, goes to `emit` (or only counts past the line limit).
    fn finish(&mut self, emit: impl FnOnce(&str)) {
        if self.current.is_empty() {
            return;
        }
        if self.logged < STDIO_STDERR_MAX_LINES {
            emit(&clean(&self.current));
            self.logged += 1;
        } else {
            self.dropped += 1;
        }
        self.current.clear();
    }
}

/// The line as text: invalid UTF-8 replaced, a trailing carriage return dropped, control
/// characters (escape sequences included) as spaces, so a line cannot forge log output.
fn clean(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.trim_end_matches('\r')
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(chunks: &[&[u8]]) -> (Vec<String>, usize) {
        let mut lines = Lines::default();
        let mut seen = Vec::new();
        for chunk in chunks {
            let mut rest: &[u8] = chunk;
            while !rest.is_empty() {
                let used = lines.take(rest, |line| seen.push(line.to_owned()));
                rest = &rest[used..];
            }
        }
        lines.finish(|line| seen.push(line.to_owned()));
        (seen, lines.dropped)
    }

    #[test]
    fn splits_lines_across_chunks() {
        let (lines, dropped) = split(&[b"one\ntw", b"o\r\n\nthree"]);
        assert_eq!(lines, ["one", "two", "three"]);
        assert_eq!(dropped, 0);
    }

    #[test]
    fn cuts_long_lines_and_counts_lines_past_the_limit() {
        let long = vec![b'a'; STDIO_STDERR_LINE_MAX_BYTES * 3];
        let (lines, _) = split(&[&long, b"\nnext\n"]);
        assert_eq!(lines[0].len(), STDIO_STDERR_LINE_MAX_BYTES);
        assert_eq!(lines[1], "next");

        let many = "x\n".repeat(STDIO_STDERR_MAX_LINES + 5);
        let (lines, dropped) = split(&[many.as_bytes()]);
        assert_eq!(lines.len(), STDIO_STDERR_MAX_LINES);
        assert_eq!(dropped, 5);
    }

    #[test]
    fn control_characters_cannot_forge_log_lines() {
        assert_eq!(clean(b"a\x1b[31mred\tb\r"), "a [31mred b");
        assert_eq!(clean(&[0xff, b'o', b'k']), "\u{fffd}ok");
    }

    #[test]
    fn the_directory_stays_inside_the_base() {
        let base = std::env::temp_dir().join(format!("copilot-stdio-{}", uuid_like()));
        let made = directory(&base, "team/files").unwrap();
        assert!(made.ends_with("team/files"));
        assert!(made.is_dir());
        for bad in ["", "../x", "/etc", "a/../../b", "."] {
            assert!(directory(&base, bad).is_err(), "{bad}");
        }
        std::os::unix::fs::symlink("/", base.join("escape")).unwrap();
        let error = directory(&base, "escape/tmp").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        std::fs::remove_dir_all(&base).unwrap();
    }

    fn uuid_like() -> String {
        format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    #[test]
    fn start_failures_are_readable() {
        let missing = start_failed(&io::Error::from(io::ErrorKind::NotFound));
        assert_eq!(missing.code(), TOOL_SERVER_START);
        assert!(missing.to_string().contains("not found"));
        let denied = start_failed(&io::Error::from(io::ErrorKind::PermissionDenied));
        assert!(denied.to_string().contains("permission denied"));
    }
}
