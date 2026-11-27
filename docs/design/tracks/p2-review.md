# P2 Review: Fixes After the Phase 2 Reviews

Branch: `claude/copilot-project-planning-6y3fep` (shared with the other checkout; merged before each push). Base: Phase 2 as integrated by P2.F. Roadmap step: `p2-review`.

## Status

Complete: P2R.1 to P2R.5. `make check`, `make test-db`, and `make test-bruno` (134 requests, 342 assertions, against the throwaway database `copilot_bruno_review` on `PORT=3031`) pass. Last green CI: `5b802f0` (Code quality, run 45929913709).

## Sub-Steps

Each finding was confirmed against the code before it was fixed.

- [x] P2R.1 rmcp's own logs (two medium findings). rmcp logs a failed connection's tool server URL (which may carry a credential in its path) and peer address at `error`, and every server notification with its contents, the server's `instructions`, and the legacy session id at `info`. `copilot-logger` now installs a `Targets` filter (`copilot_logger::targets`) that keeps the configured level for every crate and silences the crates in `SILENCED` (`rmcp`) at every level; capping at `warn`, as track C proposed, was not enough. Tests: unit tests of the filter at every `LOG_LEVEL`; `packages/mcp/tests/logs.rs` runs a failed connect with a secret in the URL path and a notification with a secret payload from servers of both eras, and checks that an unfiltered layer saw each secret (so the case still proves something) while the filtered log saw none, and that copilot-mcp's `mcp.http.failed` and `mcp.tools.called` still arrive.
- [x] P2R.2 Save as project once (medium). `ProjectRepository::insert_with_chat` moves the chat only while it is standalone (`AND project_id IS NULL`) and returns `false`, with the project rolled back, otherwise; `ChatService::save_as_project` answers `CHAT_IN_PROJECT`, or `404` when the chat was deleted meanwhile. Tests: a `make test-db` test that holds the chat row until two saves both wait on it, then expects exactly one project with the chat and its file in it; a core test with a repository that saves or deletes the chat between the service's read and its write.
- [x] P2R.3 Chat approvals only grow (low). `ChatRepository::update` replaces `chat_tool_servers` rows but no longer deletes `chat_tool_approvals`: it inserts with `ON CONFLICT DO NOTHING`, so a stale `PATCH` cannot drop an approval saved after its read. Both join inserts skip servers the user no longer has, so a stale write after a server delete no longer fails with a `500`. The in-memory store does the same. Tests: the round-trip test now expects approvals to stay; a `make test-db` test where a stale rename after an approval keeps it and skips a deleted server.
- [x] P2R.4 Tool lists of a changed URL (low). `ToolServerRepository::save_listing` takes the listed URL and stores only while the server still has it (`AND url = $7`), returning `false` otherwise. Test answers `TOOL_SERVER_CHANGED` (`422`) and stores nothing; a turn's refresh reports the server unavailable. Tests: a `make test-db` test (a listing of the old URL after a URL change stores neither its tools nor its protocol version); a core test with a tool host that moves the server while it lists, for Test and for a turn.
- [x] P2R.5 Storage quota and upload memory (low). `FileRepository::create` takes the quota and checks the user's usage again inside its transaction under `pg_advisory_xact_lock(hashtext('copilot.files.quota'), hashtext(user_id))`, returning `false` (nothing stored) past it; the service keeps its earlier check to fail before hashing and answers `STORAGE_FULL` either way. The upload routes take an `UploadSlots` slot before reading the body: `UPLOADS_PER_USER` (2) per user and `UPLOADS_AT_ONCE` (4) in all, new core constants; a request past either waits its turn (the web app sends several files at once, so refusing would break it), and dropping it frees its place. Tests: a `make test-db` test where two uploads that each fit alone but not together run at once (one is stored, nothing of the other); a core test with a stale `usage`; unit tests of the slots (per-user wait, total wait, no entry left behind, a dropped wait gives its place back).

## Not Changed

- The decisions path of `ChatService::prepare` writes the whole chat it read (`chats.update`) to add "Approve for this chat" rows, so a concurrent rename or pin could be undone by it. Approvals are safe now (they only grow); a dedicated `ChatRepository::approve` that writes approval rows only would close the rest. Done in P3.1a ([`p3-1.md`](./p3-1.md)).

## Blockers

None.
