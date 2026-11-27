# Track F: Integration

Branch: `claude/copilot-project-planning-6y3fep` (shared with the other checkout; merged before each push). Base: tracks A to D, which landed on this branch directly. Roadmap step: P2.F.

## Status

Complete: P2.F.1 to P2.F.5. Last green CI: pending on the commit that adds this file.

The roadmap's merge order (B, C, then A, with D last) and its green CI after each merge did not apply: every track committed to the shared branch, and each track recorded its own green CI run (`a.md`, `b.md`, `c.md`, `d.md`).

## Sub-Steps

- [x] P2.F.1 Real adapters in the composition root. `Adapters::create` wires `copilot-providers` (`Gateway` on `clients(Purpose::Providers)`, since P1.9) and `copilot-mcp` (`McpHost` on `clients(Purpose::Tools)`, since P2.C.2). The `Unavailable` stand-ins now compile only under `cfg(test)`, for route tests that never reach a provider or a tool server. Still deferred by the roadmap: regenerate, retry, decisions, stream reattach, stop, and `GET /api/events` answer `501`, and `MessageRepository::decide` and `recover` fail with `database.repository.unimplemented` (P3.1); Run now answers `501` (P3.2).
- [x] P2.F.2 Contract conformance test. `packages/contracts/tests/fixtures.rs` writes samples of all 77 generated contract types (every enum value, every `ChatStreamEvent`, `UserEvent`, `MessagePartDto`, and `ResultContent` variant, every optional field at least once, `PATCH` nulls) and of the 23 route bodies into `packages/contracts/test/fixtures/{types,bodies}/`; each sample must read back in Rust to the value that wrote it. `test/fixtures.test.ts` parses each sample with its Zod schema and fails when a value is rejected, dropped, or changed, when a schema declares a field, enum value, or variant no sample reaches, or when a generated type or a `*BodySchema` has no samples. Wiring: `make test` runs both (`cargo test`, then `pnpm test`); `make contracts` regenerates the fixtures with the bindings; CI (Rust job) and `make ci` fail when the committed fixtures drift, and the lint job parses them. Checked by breaking fixtures on purpose (an unknown field, a misspelled enum value, a missing variant, a renamed envelope field, a missing file): each failed. Docs: ADR-004, `AGENTS.md`, `README.md`, the `add-domain` skill, and the spec's CI table.
- [x] P2.F.3 Full suite on the integrated branch: `make check`, `make test-db` (33 API tests on Postgres, 33 repository tests, 17 schema tests, and one database unit test), `make test-bruno` (134 requests, 342 assertions, against the throwaway database `copilot_bruno_f` on `PORT=3021`, so its accounts never become the administrator of the developer's database), the web build, and `make contracts` without drift. `make migrate` ran on the developer's `copilot` database.
- [x] P2.F.4 Cross-track fixes. Track B's third request: core passed a chat's or project's reasoning level through without checking the model's `reasoning_levels`, so a project set to `off` reached Claude models that cannot turn thinking off and the provider answered `400`; `context::reasoning` now drops such a level to the model default (test `levels_the_model_does_not_list_become_the_model_default`). Track B's notes for the integrator are written into the spec's Adapter Rules, the Gemini spike outcome into Providers and Models, and the confirmed tool call id pattern into the Replay Rules.
- [x] P2.F.5 Roadmap: P2.A to P2.D ticked (their status files say complete), and P2.F.

## Open Interface-Change Requests

Carried to later serial steps; none blocks Phase 3.

- Track B: `stream_options.include_usage` for the `ollama` dialect (Ollama answers save no token counts without it); a flag on `LlmRequest` saying the model has `reasoning`, so a Claude chat with no level can still ask for summarized thinking; a larger `DEFAULT_MAX_OUTPUT_TOKENS` for reasoning models without a reported limit.
- Track C: a `TOOL_UNKNOWN` constant in core and a hook that marks a server's stored tool list stale when a call fails with it; `rmcp` logs capped at `warn` in `copilot-logger`; the adapter's time limits as core constants.
- Track D: an upload progress callback in `apps/web/src/api/client.ts` (uploads build their own `XMLHttpRequest` until then).

## Gaps

- Live checks with real keys (an Anthropic tool loop with thinking, a refusal, and cache reads; a Gemini tool loop over two turns) stay with the P3.4 owner checkpoint.
- The security reviews that the agent-workflows risk table asks for after P2.A (tenant isolation, migrations, schedules) and P2.C (outbound requests, a protocol adapter) have not run; P6.1 lists them too.
- Rustauth's DTOs are re-exported from `@rustauth/contracts` and are not part of these fixtures; lib-rustauth tests them.
- Cargo warns that `futures-core`, `mime_guess`, and `tokio-stream` are workspace dependencies no crate uses yet. `docs/research/dependencies.md` plans them (`tokio-stream` for the event streams of P3.1), so they stay.

## Blockers

None.
