# Build Roadmap

The build runs in phases. Work can stop between any two steps and resume from the pushed branches plus this file. Serial phases tick their boxes here, in the commit that finishes the step. Parallel tracks (Phase 2) keep their own status files under `docs/design/tracks/`, and the integrator ticks the Phase 2 boxes here.

Spec: [copilot.md](./copilot.md). Decisions: [ADR-005](../adr/005-copilot-architecture.md).

## How to Resume

1. `git fetch --all`, then `git branch -r --list "origin/claude/*"`. It shows the base branch plus one branch per Phase 2 track (named `claude/copilot-p2-<track>`, so the existing `claude/**` CI trigger runs on them).
2. For each branch, read `docs/design/tracks/<track>.md` (Phase 2) or the boxes below (serial phases), and run `git log --oneline -5`. Commit subjects use the step id as scope (`feat(p2.b): ...`) plus a `Roadmap-Step: P2.B.3` trailer, so `git log --grep "Roadmap-Step: P2.B"` finds a step's commits.
3. Check CI on each branch head (`gh run list --branch <branch> --limit 1`). A box ticked on a commit with red CI is not done: fix CI first.
4. Only pushed work exists. Agents commit and push after every finished sub-step, and push a `wip(<step>): ...` commit before stopping. In a cloud session, unpushed work is lost when the container ends.
5. A step marked **STOP** stays blocked until the developer writes "Reviewed" in its status line (in this file or the track file). Meanwhile, continue with other unblocked steps.
6. Continue with the first unticked, unblocked box of the earliest phase.

## Definition of Done

Every step needs all of these:

- `make check` green locally.
- The commit pushed, and CI green on that commit. CI is where DB tests and Bruno run for agents without Postgres; say so in the commit message when they were not run locally.
- Steps that change contracts also pass `make ci` (contracts drift check).
- The step's own tests below.
- Owner checkpoints (live keys, devices, click-through) are explicit boxes; agents do not tick them.

## Phase 0: Bootstrap

- [x] P0.1 Copy the RustInit template, rename to Copilot, CI on branch pushes
- [x] P0.2 Research notes in `docs/research/` (voice, providers, MCP, native apps and CI, dependencies)
- [ ] P0.3 Design spec reviewed (six lenses) and revised (`docs/design/copilot.md` v2); ADR-005 written. **STOP**: developer reviews the spec, the data model, and ADR-005.
- [ ] P0.4 Delivery tooling:
  - DB tests use `#[sqlx::test(migrator = "copilot_database::operations::migrate::MIGRATOR")]` plus `#[ignore = "requires Postgres: run make test-db"]`; `make test-db` exports `DATABASE_URL` pointing at the `postgres` maintenance database, so every test gets a throwaway database and never touches `copilot`. sqlx with `migrate` and `macros` as a dev-dependency where needed; the add-domain skill's test step updated.
  - CI migration immutability guard (see the spec's CI section) and the CI split into a slim lint job and a Rust plus Postgres job.
  - `scripts/worktree.sh <track>`: `git worktree add ../copilot-<track> -b claude/copilot-p2-<track> p1-frozen`, `pnpm install --frozen-lockfile`, copy the env samples, set a distinct `PORT` (3001 and up) and Bruno base URL. The owner runs it once per track, or approves adding `Bash(bash scripts/worktree.sh *)` to the allow list.
  - `scripts/test-bruno.sh` refuses to reuse a running server unless `BASE_URL` is set explicitly.
  - `docs/design/tracks/README.md` with the ownership table below.
  - Tests: CI green on the pushed commit; the guard fails on a test branch that edits an existing migration.

## Phase 1: Foundation and Walking Skeleton

Serial, one agent, on the base branch. This phase builds every shared file once, so Phase 2 tracks rarely touch them.

- [ ] P1.1 Workspace: every workspace dependency from `docs/research/dependencies.md` plus the rmcp line (client and Streamable HTTP features only); every web package; `exclude = ["apps/native/src-tauri"]`; crates `copilot-providers`, `copilot-mcp`, `copilot-security` scaffolded with the boundary rules from the spec; Lambda removed (`bin/lambda.rs`, the `lambda` feature and `[[bin]]` entry, `lambda_http`, `docs/deployment-lambda.md`); `AUTH_DISABLED`, `NoopTokenVerifier`, `RejectingTokenVerifier` removed; AGENTS.md package map updated. Tests: `cargo tree -i aws-lc-rs` prints nothing; record release binary size and crate count as the footprint baseline in the commit message.
- [ ] P1.2 Core domains, one box each, in this order. Each: entities, value objects, ports, services, fakes behind the `testing` feature, unit tests.
  - [ ] P1.2a `identity` (registration rules, throttle, session sliding and absolute expiry, last-admin rule)
  - [ ] P1.2b `provider` (capability defaults, locks, dialects, model sync rules)
  - [ ] P1.2c `project`
  - [ ] P1.2d `file` (allowlist, magic bytes, quota, token estimate)
  - [ ] P1.2e `tool` (aliases, tool modes, changed-tool detection)
  - [ ] P1.2f `schedule` (`CronRule`, min interval, preview, `fire_due` with compare-and-set, failure counting)
  - [ ] P1.2g `chat` (prepare, save as project, tail edits)
- [ ] P1.3 Conversation engine in core against the fakes: `context::build`, replay rules, approval policy, `ChatEngine::run`, built-in schedule tools. This freezes `LlmGateway`, `ToolHost`, `TurnSink`, and `MessageRepository`. Tests: text, reasoning, tool loop, `MAX_TOOL_ROUNDS` stop with `tool_limit`, approval pause and continuation from the snapshot, cancel persisting `cancelled` plus synthesized results, provider error persisting `error`, refusal, replay only when provider and `context_hash` match, lowering placeholders, window trimming.
- [ ] P1.4 Migrations derived from the entities, one per domain, transcribed from the spec's Data Model; `SCHEMA_TABLES` registered with sealed and hash columns omitted. Tests: up, down to zero, up again; the readonly role cannot select `password_hash`, `token_hash`, `api_key_sealed`, `headers_sealed`, or any `messages` row; cascades and `SET NULL` behave as specified. **STOP**: developer reviews the migrations.
- [ ] P1.5 Contracts for every route, `ChatStreamEvent` and `UserEvent`; Zod schemas; `make contracts`.
- [ ] P1.6 `copilot-security`: sealing (format and AAD), Argon2id behind the semaphore, tokens, the two hardened outbound clients with the DNS resolver; the core address policy. Tests: seal and open round trip; wrong key and tampered ciphertext fail; wrong AAD fails; table-driven address tests (IPv4-mapped IPv6, CGNAT, link-local and metadata always blocked, IP literals).
- [ ] P1.7 API skeleton: every env var in `AppConfig`, `docs/env.md`, and `.env.sample`; `AppState` with every service; composition root wiring the database and security adapters plus `Unavailable` stubs for `LlmGateway` and `ToolHost`; every route module declared under `/api`; SPA serving from `WEB_DIR`; identity routes, session `TokenVerifier`, login throttling, setup token; Dockerfile and `deploy/compose.yml`; Bruno `02 Auth` (registers a run-unique user, stores the token for later folders).
- [ ] P1.8 Web foundation: `apiFetch` (method, JSON body, bearer token, injectable fetch, runtime server URL for native builds); SSE-over-POST reader (`reader.read()` loop); router with every path constant and a placeholder page; auth screens and first-run setup shell; web app manifest; dev proxy for `/api`; a TS test runner (Rstest or similar) with tests for `apiFetch` and the SSE reader.
- [ ] P1.9 Walking skeleton: OpenAI-compatible text-only streaming in `copilot-providers`, `POST /api/chats/{id}/messages` through `ChatService::prepare`, `ChatEngine`, and a minimal `RunRegistry`, and a minimal chat page. Tag `p1-frozen`.

### Freeze List

After `p1-frozen`, a track changes these only through a serial interface-change step on the base branch (request it in the track's status file), which every track then rebases onto:

1. `LlmGateway`, `LlmRequest`, `LlmEvent`, `ModelTarget`, `ToolHost`, `ToolSession`, `TurnSink`, `TurnEvent`, and the repository port signatures in the spec.
2. The stored JSON records (`PartRecord`, `provider_state`, `UsageRecord`, `CapabilitiesRecord`), all versioned with `v`.
3. The outbound client builder in `copilot-security`; adapters never build their own client.
4. `Clock`, `SecretCipher`, the `DomainError` variants and their `HttpError` mapping (`Unauthorized`, `RateLimited`, `Upstream`).
5. Core fakes behind the `testing` feature.
6. Every env var, `AppState` field, route module, contract type, workspace dependency, and web path constant (all created as stubs in Phase 1).

## Phase 2: Parallel Tracks

Worktrees branched from `p1-frozen` (`scripts/worktree.sh <track>`). Tracks never edit this file; each keeps `docs/design/tracks/<track>.md` with sub-step boxes, status, blockers, and interface-change requests.

| Track | Owns |
| --- | --- |
| A: API and repositories | `packages/database/src/repositories/**`, migrations and `SCHEMA_TABLES` (the only track that adds migrations), `apps/api/src/routes/<domain>*.rs`, `apps/api/tests/bruno/03-*` and later |
| B: Providers | `packages/providers/**` |
| C: MCP | `packages/mcp/**` |
| D: Web | `apps/web/src/routes/**`, `apps/web/src/components/**`, `apps/web/src/api/routes/**` |
| Frozen for all tracks | Root `Cargo.toml` and `Cargo.lock`, `packages/core/**`, `packages/contracts/**`, `apps/api/src/{lib.rs,config/**,helpers/bootstrap.rs,routes/mod.rs}`, `packages/middleware/**`, `docs/env.md`, `apps/api/.env.sample`, `apps/web/src/{api/client.ts,api/endpoints.ts,config.ts,routes/index.tsx,routes/path.tsx}`, `apps/web/rsbuild.config.ts`, `pnpm-lock.yaml`, `.github/workflows/**` |

When generated contract files conflict, rerun `make contracts`; never merge them by hand.

- [ ] P2.A API and repositories, one sub-box per domain in the track file: users and sessions (admin routes, devices), providers and models, projects, files (multipart, content headers, quota), chats CRUD, read state and search (no streaming), tool servers, schedules CRUD and preview. Tests: `#[sqlx::test]` repository tests (first-user-admin race, closed registration, last admin, compare-and-set claim), Bruno folder per domain including the cross-user 404 check.
- [ ] P2.B Providers: OpenAI-compatible tools, reasoning fields, dialects, usage, audio parts; Anthropic adapter (thinking, signatures, caching, refusal); model listing and capability defaults; the Gemini spike through the OpenAI-compatible endpoint, with its outcome recorded in ADR-005. Tests: wiremock fixtures per provider for text, every reasoning field variant, tool-call argument deltas, replay state, usage, every finish-reason mapping, HTTP and mid-stream errors.
- [ ] P2.C MCP client: dual-era rmcp over the injected client, tool listing with `ttlMs`, response caps, timeouts, unknown-tool refresh, era memory. Tests: an in-process rmcp server (dev-dependency) in both eras; timeout; truncation; unknown-tool cache refresh.
- [ ] P2.D Web: Settings (account, devices, providers and models, tool servers, users), first-run setup, projects and files with the capacity bar, sidebar and chat list, search, non-streaming chat view. Tests: TS unit tests for stores and request helpers.
- [ ] P2.F Integration (serial, base branch): merge B, C, then A; replace the stubs in the composition root; merge D. Run `make check` and wait for green CI after each merge. Tick the P2 boxes here only.

## Phase 3: Conversation Features

- [ ] P3.1 Full run registry and chat flows: single-flight and per-user limits, `seq` and reattach replay, stop, graceful shutdown and startup recovery, decisions route and continuation, regenerate, edit and resend, Continue, MCP tools in turns, `GET /api/events`. Tests: API integration tests with wiremock and the rmcp server covering reattach replay, "disconnect does not cancel", stop, approval pause and resume, new message while pending, restart recovery.
- [ ] P3.2 Scheduler loop (tick, compare-and-set claim, at most 2 runs, missed fires collapse), failure counting and auto-disable, model-removed pause, built-in schedule tools exposed to the model. Tests: DB tests where two concurrent claimers fire a due schedule exactly once; DST gap and overlap; unattended runs never see schedule tools.
- [ ] P3.3 Web streaming, tool cards and approval cards, schedules UI (Scheduled list, form with presets and preview), unread and needs-approval dots, platform behavior (Enter rules, back handling, shortcuts). May run in a worktree in parallel with P3.2 and owns `apps/web` only. Tests: TS tests for the stream reducer.
- [ ] P3.4 Owner checkpoint: click-through on `make dev` and a live smoke test against at least one OpenAI-compatible and one Anthropic connection.

## Phase 4: Voice

- [ ] P4.1 Audio parts: `input_audio` for OpenAI-compatible models (and Gemini if the spike confirmed it); audio out (`audio.delta`, transcript as text, WAV saved as a chat file, Speak replies toggle). Tests: wiremock fixtures for audio deltas and transcripts.
- [ ] P4.2 Web push to talk (AudioWorklet, 16 kHz mono WAV, 5-minute cap) and playback of streamed and saved audio.

## Phase 5: Native Apps

- [ ] P5.1 `apps/native` Tauri v2 scaffold wrapping the web build; `tauri-plugin-http` wired to the injectable fetch; connect-to-server screen with the plain-HTTP warning; CSP and opener scope; `pnpm-workspace.yaml`.
- [ ] P5.2 Native CI workflows (`native-linux` on pinned `ubuntu-24.04`, `native-android`), run on tags, manual dispatch, and changes under `apps/native`. Tests: CI artifacts for the `.deb` and the arm64 APK.
- [ ] P5.3 Microphone on Android (manifest permissions) and Linux (media stream setting, microphone-only permission handler).
- [ ] P5.4 Owner checkpoint: device test per `docs/research/native-and-ci.md` section 12, plus the Android file picker, camera capture, and back button.

## Phase 6: Hardening

- [ ] P6.1 Multi-lens review (correctness, security, layering rules, UX, footprint) and fixes. Security also reviews P1.6, P1.7, P2.A, and P2.C at their own phase gates.
- [ ] P6.2 Docs: README, `docs/env.md`, self-hosting with compose and a reverse proxy, ADR updates.
- [ ] P6.3 CI green on GitHub; footprint measured against the P1.1 baseline.

## Notes for the Developer

- **Migration policy** (matches AGENTS.md, ADR-002, and `.claude/settings.json`): assistants may write migrations and run `make migrate`. Agents verify schema work on CI's fresh database and on `#[sqlx::test]` databases, never by waiting for the developer.
- A migration is immutable once committed, whoever applied it; fix mistakes with a new migration. The CI guard enforces this.
- In Phase 2 only track A adds migrations; other tracks file an interface-change request.
- Apply migrations to the developer's database only from the main checkout, after an integration merge; running `make migrate` from a worktree against the shared database breaks the other worktrees.
- `make test-db` and `make test-bruno` need Postgres; CI runs them on every push. Track branches rely on CI for Bruno.
- Each worktree has a cold `target/` directory. A shared `CARGO_TARGET_DIR` saves disk and build time but serializes builds across worktrees.
