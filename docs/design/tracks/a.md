# Track A: API and Repositories

Branch: `claude/copilot-project-planning-6y3fep` (shared with the other Phase 2 track that runs at the same time; each checkout merges the branch before it pushes). Base: `p1-frozen`. Roadmap step: P2.A.

## Status

Part 1 done (P2.A.1 to P2.A.4). Part 2 in progress: P2.A.5 (chats) and P2.A.6 (search) done; tool servers and schedules open. Last green CI: `a3b30cb` (the merge with P2.A.4).

## Sub-Steps

Part 1: settings, providers and models, projects, files. Part 2: chats, read state and search, tool servers, schedules.

- [x] P2.A.1 User settings: `GET` and `PATCH /api/settings`. Tests: repository upsert test (P1.9), route test on Postgres (a user reads empty instructions, a `PATCH` saves them, another user still reads empty ones), Bruno `03 Settings` with the cross-user check.
- [x] P2.A.2 Providers and models: `GET`, `PATCH`, and `DELETE /api/providers/{id}` (details with `usage`), `ProviderRepository::usage`. Tests: repository test of the usage counts (only the caller's chats, projects, and schedules that name one of the connection's models), route test on Postgres (update clears the key on a new origin, delete disables schedules and leaves chats without a model), Bruno `04 Providers` with the cross-user check in paths.
- [x] P2.A.3 Projects: `GET` and `POST /api/projects`, `GET`, `PATCH`, and `DELETE /api/projects/{id}` (details with `capacity`), every `ProjectRepository` method including `insert_with_chat`; the delete handler cancels live runs of the project's chats first. Tests: repository tests (insert and update with the tool server rows in one transaction, list order and filters, delete cascades, save as project moves the chat and its attachments), route tests on Postgres (another user's model and tool server ids in bodies, delete stopping a live run), Bruno `05 Projects` with the cross-user check in paths and in bodies (a chat in another user's project).
- [x] P2.A.4 Files: multipart uploads to projects and chats (allowlist, magic bytes, size and quota), list, `PATCH`, move, delete, and `GET /api/files/{id}/content` with the safety headers. Tests: repository tests (create in one transaction, list, content, update, move, delete, usage), route tests (headers, `filename*` encoding, served type, SVG and HTML never served as their own types, multipart field errors), Bruno `06 Files` with the cross-user check in paths and bodies.

- [x] P2.A.5 Chats: `GET /api/chats` (filters `projectId`, `scheduleId`, `pinned`, `scheduled`, `limit`, and the cursor), `PATCH` and `DELETE /api/chats/{id}` (the delete stops a live run first), `POST /api/chats/{id}/read`, `save-as-project`, and `save-instructions`; `ChatRepository::list` (newest first by `(updated_at, id)`, with the unread and awaiting-approval flags), `MessageRepository::replace_tail` and `delete_after`. Tests: repository tests (paging through ties, every filter alone and combined, the unread flag, the awaiting flag of the latest answer only, tail deletes scoped to the chat and owner, a failed replace rolling back), cursor unit tests, route tests on Postgres (list, paging, filters, invalid cursor, settings changes that do not move a chat, read state, delete, another user's ids in paths, queries, and bodies, save as project and save instructions, edit and resend, a delete stopping a live run), Bruno `07 Chats` with the cross-user checks in paths, queries, and bodies.
- [x] P2.A.6 Search: `GET /api/search` (`q`, `limit`) and `MessageRepository::search`, full-text over chat titles and message `text_content` with the `simple` configuration (the GIN index's expression). Every word of the search text matches as a prefix; title matches come first (newest chat first), then messages by rank, newest first; snippets are plain text on one line. Tests: repository test (titles before messages, prefixes, every word required, case, limit, text without words, query operators as plain text, another user's chats never found), route test on Postgres (another user's search, blank, missing, and too long text), Bruno `08 Search` with the cross-user check.

## Notes

- Two tests in frozen files name routes that answered `501`: `routes::tests::signed_in_callers_reach_the_handlers` in `apps/api/src/routes/mod.rs` lists the written operations, and `helpers::auth::tests::starts_on_postgres_once_the_auth_schema_is_migrated` in `apps/api/src/helpers/auth.rs` expected `501` from `GET /api/settings`. Each sub-step updates those test lines only and changes nothing else in the files.
- `apps/api` now depends on `serde` (for `routes::path::Params`, the query string extractor with the shared error body), which adds one line to `Cargo.lock`.
- `RunRegistry` (`helpers/runs.rs`) gained `chats(owner)` and `stop(chat, wait)`, which cancels a run and waits for its slot to be released; deleting a project stops the live runs of its chats with it, and P3.1's stop route can use it.
- The file repository landed with P2.A.3, because a project's details read its files for the capacity bar.
- File downloads also send `Cache-Control: private, no-store` (beyond the spec's list), so no shared cache keeps a user's bytes and a deleted file does not linger.
- Uploads are read in chunks and refused once they pass `MAX_UPLOAD_MB`; the upload routes allow bodies up to the largest accepted `MAX_UPLOAD_MB` (1000) plus 1 MiB of framing (`DefaultBodyLimit`), and a part other than one named `file` is a `400`. The storage quota is checked before the write (`FileService::upload`), so concurrent uploads of one user can pass it by at most the uploads in flight.
- Bruno upload fixtures live in `apps/api/tests/bruno/fixtures/`.
- Bruno has no model provider to refresh against, so its folders hold no models and no tool servers (part 2 writes their routes). The cross-user checks of model and tool server ids in bodies are route tests on Postgres until then; Bruno checks project ids in bodies (`POST /api/chats`, `POST /api/files/{id}/move`).
- The chat list cursor is opaque to clients: unpadded URL-safe base64 of `<updated_at, RFC 3339 with microseconds>_<id>` (`routes/chats/cursor.rs`). A cursor the server did not write is a `400` on the `cursor` field. It is written without naming `chrono`, so `apps/api` needs no new dependency.
- The chat list's awaiting-approval flag reads the latest assistant message only, like the in-memory fake; a `tool` message after it does not clear the flag.
- Search builds its `tsquery` in Postgres: `plainto_tsquery('simple', q)` (which accepts any text), then `:*` after each quoted lexeme, so operators in the search text are plain text. The in-memory fake matches substrings instead.
- `MessageRepository::replace_tail` and `delete_after` landed with P2.A.5: editing a message (`editFrom` on `POST /api/chats/{id}/messages`) reached `replace_tail`, which answered `500` until then. `decide` and `recover` stay for P3.1.
- Bruno runs locally against a throwaway database (`DB_NAME=copilot_bruno_a make migrate`, then `DB_NAME=copilot_bruno_a PORT=3017 make test-bruno`), so its accounts never become the administrator of the developer's `copilot` database.

## Blockers

None.

## Interface-Change Requests

Both tracks share one branch, so these landed directly, each in the commit that needed it:

- `apps/api` enables axum's `multipart` feature (planned in `docs/research/dependencies.md`), which adds `multer` and `encoding_rs` (with two small helpers) to `Cargo.lock`. Landed with P2.A.4.
- `apps/api` depends on `serde` directly (see Notes). Landed with P2.A.3.
