# Phase 2 Tracks

Phase 2 of the [roadmap](../roadmap.md) runs four tracks in parallel, each in its own git worktree and branch, all starting from the `p1-frozen` tag. This folder holds one status file per track (`a.md`, `b.md`, `c.md`, `d.md`) and one for the integration step (`f.md`, with the interface-change requests still open after Phase 2), plus `p2-review.md` for the fixes after the Phase 2 reviews, `p3-1.md` for the run registry and chat flows of Phase 3, `p3-2.md` for the scheduler loop and Run now, `p3-3.md` for the web streaming and conversation features, `p4-1.md` for audio parts and saved spoken replies on the server, `p4-2.md` for push to talk and spoken reply playback in the web client, `p4-3.md` for the fake model server and the smoke test, `p5-1.md` for the Tauri shell, `p5-2.md` for the native CI workflow, `p5-3.md` for the microphone in the native shells, and `p45-review.md` for the fixes after the reviews of Phases 4 and 5. Tracks never edit the roadmap; the integrator (P2.F) ticks the Phase 2 boxes there after merging.

## Ownership

Each track edits only the paths it owns. Everything in the frozen row changes only through a serial interface-change step on the base branch (see [Interface Changes](#interface-changes)).

| Track | Owns |
| --- | --- |
| A: API and repositories | `packages/database/src/repositories/**`, migrations and `SCHEMA_TABLES` (the only track that adds migrations), `apps/api/src/routes/<domain>*.rs`, `apps/api/tests/bruno/03-*` and later |
| B: Providers | `packages/providers/**` |
| C: MCP | `packages/mcp/**` |
| D: Web | `apps/web/src/routes/**`, `apps/web/src/components/**`, `apps/web/src/api/routes/**` |
| Frozen for all tracks | Root `Cargo.toml` and `Cargo.lock`, `packages/core/**`, `packages/contracts/**`, `apps/api/src/{lib.rs,config/**,helpers/auth.rs,helpers/bootstrap.rs,routes/mod.rs}`, `packages/middleware/**`, `docs/env.md`, `apps/api/.env.sample`, `apps/web/src/{api/client.ts,api/endpoints.ts,config.ts,routes/index.tsx,routes/path.tsx}`, `apps/web/rsbuild.config.ts`, `pnpm-lock.yaml`, `.github/workflows/**` |

The rustauth pin (in the root `Cargo.toml`, `Cargo.lock`, and `pnpm-lock.yaml`) is frozen with them. A track that needs a change in lib-rustauth files an interface-change request; the change lands in lib-rustauth first, then as a pin bump on the base branch.

When generated contract files conflict, rerun `make contracts`; never merge them by hand.

## Starting a Track

From the main checkout, once `p1-frozen` exists:

```bash
bash scripts/worktree.sh a
```

This creates `../copilot-a` on branch `claude/copilot-p2-a` from `p1-frozen`, runs `pnpm install --frozen-lockfile`, creates the env files from `apps/api/.env.sample` and `apps/web/.env.example` (`scripts/env-local.sh`, which also generates a `SECRET_KEY`), and sets a distinct API `PORT` (3001 and up) plus the matching `PUBLIC_API_URL`. `make test-bruno` in the worktree starts that worktree's API and tests its port; it refuses to reuse a server that is already running unless `BASE_URL` is set explicitly.

Then create the track's status file from the template below and push the branch.

Notes for worktrees:

- Run `make migrate` only from the main checkout, after an integration merge. Running it from a worktree against the shared database breaks the other worktrees. `make test-db` is safe everywhere: each test gets its own throwaway database.
- Each worktree has a cold `target/` directory. A shared `CARGO_TARGET_DIR` saves disk and build time but serializes builds across worktrees.
- Track branches rely on CI for Bruno when no local Postgres is running.

## Status File Template

Copy this into `docs/design/tracks/<track>.md` (for example `a.md`) on the track branch:

```markdown
# Track A: API and Repositories

Branch: `claude/copilot-p2-a`. Base: `p1-frozen`. Roadmap step: P2.A.

## Status

In progress: <sub-step>. Last green CI: <commit>.

## Sub-Steps

- [ ] P2.A.1 <sub-step>. Tests: <what proves it>.
- [ ] P2.A.2 <sub-step>. Tests: <what proves it>.

## Blockers

None.

## Interface-Change Requests

None.
```

Commit subjects use the sub-step as scope (`feat(p2.a): ...`) plus a `Roadmap-Step: P2.A.3` trailer, so `git log --grep "Roadmap-Step: P2.A"` finds a step's commits. Tick a sub-step box in the commit that finishes it, and push after every finished sub-step.

## Interface Changes

A track that needs a change to a frozen path (a port signature, a contract type, an env var, a workspace dependency) does not make it on its branch. It adds an entry under "Interface-Change Requests" in its status file with the change and the reason, pushes, and continues with work that does not depend on it. The change then lands as a serial step on the base branch, and every track rebases onto it.
