#!/usr/bin/env python3
"""Pre-tool hook for Claude Code and Cursor: keep committed migrations immutable.

Policy (see .ai/rules/no-migration-edits.md):

- Files under packages/database/migrations/ that exist in the last commit are read-only for AI
  assistants: no edits, renames, or deletes. sqlx checksums applied migrations, so changing one
  breaks every database that already ran it.
- New, uncommitted migration files may be written (scaffold them with `make migrations`).
- Applying migrations is the developer's step: `make migrate`, the `migrate` binary, and
  `sqlx migrate run|revert` are blocked.
- Shell commands that name the migrations folder are blocked; use the Read/Glob tools to inspect
  it and the Write/Edit tools for new files.

Claude Code: reads the PreToolUse payload on stdin; exit 2 blocks and stderr goes back to Claude.
Cursor: prints {"permission": "allow" | "deny", ...} on stdout and exits 2 on deny.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys

MIGRATIONS_DIR = "packages/database/migrations"

EDIT_MESSAGE = (
    "Committed migrations are read-only: sqlx checksums every applied migration. "
    "Add a new migration instead (`make migrations name=<name>`, then write SQL in the new "
    "uncommitted files) and let the developer review and run `make migrate`."
)
APPLY_MESSAGE = (
    "Applying migrations is the developer's step. Tell the user to review the new migration "
    "and run `make migrate`."
)
SHELL_PATH_MESSAGE = (
    "Do not run shell commands against packages/database/migrations/. Use the Read or Glob "
    "tools to inspect it and Write/Edit for new, uncommitted migration files."
)

APPLY_PATTERNS = (
    re.compile(r"\bmake\b[^\n;&|]*\bmigrate\b"),
    re.compile(r"--bin[\s=]+migrate\b"),
    re.compile(r"\btarget/(?:debug|release)/migrate\b"),
    re.compile(r"\bsqlx\s+migrate\s+(?:run|revert)\b"),
    re.compile(r"\bsqlx\s+database\s+(?:drop|reset)\b"),
)

CLAUDE_FILE_TOOLS = {"Write", "Edit", "MultiEdit", "NotebookEdit"}
CURSOR_FILE_TOOLS = {"Write", "Delete", "StrReplace", "Edit", "MultiEdit"}


def project_root() -> str:
    root = os.environ.get("CLAUDE_PROJECT_DIR") or os.environ.get("CURSOR_PROJECT_DIR")
    if root:
        return root
    return os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def relative_path(path: str, root: str) -> str:
    normalized = path.replace("\\", "/")
    if os.path.isabs(normalized):
        normalized = os.path.relpath(normalized, root).replace("\\", "/")
    return normalized.removeprefix("./")


def is_migration_path(relative: str) -> bool:
    return relative == MIGRATIONS_DIR or relative.startswith(MIGRATIONS_DIR + "/")


def is_committed(relative: str, root: str) -> bool:
    """True when the file exists in HEAD. Fails closed (True) if git cannot answer."""
    try:
        result = subprocess.run(
            ["git", "-C", root, "cat-file", "-e", f"HEAD:{relative}"],
            capture_output=True,
            check=False,
            timeout=10,
        )
    except (OSError, subprocess.SubprocessError):
        return True
    if result.returncode == 0:
        return True
    # No commits yet, or the path is not in HEAD: a new file.
    stderr = result.stderr.decode(errors="replace")
    return "Not a git repository" in stderr or "not a git repository" in stderr


def file_denial(path: str, root: str) -> str | None:
    if not path:
        return None
    relative = relative_path(path, root)
    if is_migration_path(relative) and is_committed(relative, root):
        return EDIT_MESSAGE
    return None


def shell_denial(command: str) -> str | None:
    if any(pattern.search(command) for pattern in APPLY_PATTERNS):
        return APPLY_MESSAGE
    if MIGRATIONS_DIR in command.replace("\\", "/"):
        return SHELL_PATH_MESSAGE
    return None


def parse_tool_input(raw: object) -> dict:
    if isinstance(raw, dict):
        return raw
    if isinstance(raw, str):
        try:
            parsed = json.loads(raw)
        except json.JSONDecodeError:
            return {}
        return parsed if isinstance(parsed, dict) else {}
    return {}


def handle_claude(data: dict, root: str) -> None:
    tool_name = data.get("tool_name", "")
    tool_input = parse_tool_input(data.get("tool_input"))

    message = None
    if tool_name in CLAUDE_FILE_TOOLS:
        path = tool_input.get("file_path") or tool_input.get("notebook_path") or ""
        message = file_denial(path, root)
    elif tool_name == "Bash":
        message = shell_denial(tool_input.get("command", ""))

    if message:
        print(message, file=sys.stderr)
        sys.exit(2)
    sys.exit(0)


def cursor_reply(message: str | None) -> None:
    if message:
        json.dump(
            {"permission": "deny", "user_message": message, "agent_message": message},
            sys.stdout,
        )
        sys.exit(2)
    json.dump({"permission": "allow"}, sys.stdout)
    sys.exit(0)


def handle_cursor(data: dict, root: str) -> None:
    if data.get("hook_event_name") == "beforeShellExecution":
        cursor_reply(shell_denial(data.get("command", "")))

    tool_name = data.get("tool_name", "")
    tool_input = parse_tool_input(data.get("tool_input"))
    if tool_name in CURSOR_FILE_TOOLS:
        path = tool_input.get("path") or tool_input.get("file_path") or ""
        cursor_reply(file_denial(path, root))
    if tool_name == "Shell":
        cursor_reply(shell_denial(tool_input.get("command", "")))
    cursor_reply(None)


def main() -> None:
    try:
        data = json.load(sys.stdin)
    except json.JSONDecodeError:
        sys.exit(0)
    if not isinstance(data, dict):
        sys.exit(0)

    root = project_root()
    if data.get("hook_event_name") == "PreToolUse":
        handle_claude(data, root)
    handle_cursor(data, root)


if __name__ == "__main__":
    main()
