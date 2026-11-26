# Track A: API and Repositories

Branch: `claude/copilot-project-planning-6y3fep` (shared with the other Phase 2 track that runs at the same time; each checkout merges the branch before it pushes). Base: `p1-frozen`. Roadmap step: P2.A.

## Status

In progress: P2.A.3 projects. Last green CI: none yet on this track.

## Sub-Steps

Part 1 (this run): settings, providers and models, projects, files. Part 2 (a later run): chats, read state and search, tool servers, schedules.

- [x] P2.A.1 User settings: `GET` and `PATCH /api/settings`. Tests: repository upsert test (P1.9), route test on Postgres (a user reads empty instructions, a `PATCH` saves them, another user still reads empty ones), Bruno `03 Settings` with the cross-user check.
- [x] P2.A.2 Providers and models: `GET`, `PATCH`, and `DELETE /api/providers/{id}` (details with `usage`), `ProviderRepository::usage`. Tests: repository test of the usage counts (only the caller's chats, projects, and schedules that name one of the connection's models), route test on Postgres (update clears the key on a new origin, delete disables schedules and leaves chats without a model), Bruno `04 Providers` with the cross-user check in paths.
- [ ] P2.A.3 Projects: `GET` and `POST /api/projects`, `GET`, `PATCH`, and `DELETE /api/projects/{id}` (details with `capacity`), every `ProjectRepository` method including `insert_with_chat`; the delete handler cancels live runs of the project's chats first. Tests: repository tests (insert and update with the tool server rows in one transaction, list order and filters, delete cascades, save as project moves the chat and its attachments), route test on Postgres, Bruno `05 Projects` with the cross-user check in paths and bodies (another user's tool server and model ids).
- [ ] P2.A.4 Files: multipart uploads to projects and chats (allowlist, magic bytes, size and quota), list, `PATCH`, move, delete, and `GET /api/files/{id}/content` with the safety headers. Tests: repository tests (create in one transaction, list, content, update, move, delete, usage), route tests (headers, `filename*` encoding, served type, SVG and HTML never served as their own types, multipart field errors), Bruno `06 Files` with the cross-user check in paths and bodies.

## Notes

- The route test `routes::tests::signed_in_callers_reach_the_handlers` in `apps/api/src/routes/mod.rs` (a frozen file) lists the written operations; each sub-step adds its routes to that test-only list and changes nothing else in the file.
- Bruno has no model provider to refresh against, so its folders hold no models and no tool servers (part 2 writes their routes). The cross-user checks of model and tool server ids in bodies are route tests on Postgres until then; Bruno checks project ids in bodies (`POST /api/chats`, `POST /api/files/{id}/move`).
- Bruno runs locally against a throwaway database (`DB_NAME=copilot_bruno_a make migrate`, then `DB_NAME=copilot_bruno_a PORT=3017 make test-bruno`), so its accounts never become the administrator of the developer's `copilot` database.

## Blockers

None.

## Interface-Change Requests

None.
