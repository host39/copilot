# Build Roadmap

The build runs in phases. Each phase ends with a commit pushed to the working branch, so work can stop between phases (or between the steps inside one) and resume from `git log` plus this file. Tick a box in the same commit that finishes the step.

## How to Resume

1. `git log --oneline` shows the last finished step (commit subjects start with the phase id, for example `feat(p2-providers): ...`).
2. Find the first unticked box below and continue from there.
3. Uncommitted changes in the working tree belong to the interrupted step: review them, then finish or discard that step.

## Phase 0: Bootstrap

- [x] P0.1 Copy the RustInit template, rename to Copilot, CI on branch pushes
- [ ] P0.2 Research notes in `docs/research/` (voice, providers, MCP, native apps and CI, dependencies)
- [ ] P0.3 Design spec reviewed (`docs/design/copilot.md`) and ADR-005 written

## Phase 1: Foundation (Interfaces First)

- [ ] P1.1 Schema migrations for every table in the design
- [ ] P1.2 Core domains: entities, ports, services, in-memory fakes, unit tests
- [ ] P1.3 Contracts for every route and the chat event stream, Zod schemas, `make contracts`
- [ ] P1.4 New crates scaffolded (`copilot-providers`, `copilot-mcp`, `copilot-security`, `copilot-storage`) with boundary rules
- [ ] P1.5 Review and fix pass, `make check` green

## Phase 2: Parallel Tracks

- [ ] P2.1 Database repositories, API routes, composition root, Bruno tests (identity, providers, models, projects, files, chats, tool servers, schedules)
- [ ] P2.2 `copilot-providers`: OpenAI-compatible, Anthropic, Gemini streaming adapters with mock-server tests
- [ ] P2.3 `copilot-mcp`: MCP client adapter
- [ ] P2.4 `copilot-security` and `copilot-storage` adapters
- [ ] P2.5 Web UI: auth, layout, settings, projects, chat (against contracts)
- [ ] P2.6 Native app scaffold (`apps/native`, Tauri v2) and CI workflows
- [ ] P2.7 Merge tracks, `make check` green

## Phase 3: Conversation Engine

- [ ] P3.1 Chat engine wired end to end: streaming, tool loop, approvals, run registry, reattach, stop
- [ ] P3.2 Scheduler loop and built-in schedule tools
- [ ] P3.3 Web chat streaming, tool cards, schedules UI

## Phase 4: Voice

- [ ] P4.1 Audio in and out for providers that support it natively (per `docs/research/voice.md`)

## Phase 5: Hardening

- [ ] P5.1 Multi-lens review (correctness, security, layering rules, UX, footprint) and fixes
- [ ] P5.2 Docs: README, env reference, deployment, ADRs
- [ ] P5.3 CI green on GitHub, footprint measured

## Notes for the Developer

- Migrations are written by the assistant but **applied only by you**: review `packages/database/migrations/` and run `make migrate`.
- `make test-db` and `make test-bruno` need Postgres; CI runs them on every push to the branch.
