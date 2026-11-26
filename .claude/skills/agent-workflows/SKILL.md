---
name: agent-workflows
description: The owner's standing configuration for multi-agent work (workflows, subagents, parallel tracks, reviews and verification, model choice, CI runners). Use when planning or running a workflow or subagents, deciding how to review or verify work, picking a model for an agent, splitting work into phases, or writing GitHub Actions jobs.
---

# Agent Workflows

The owner's standing decisions for building with multiple AI agents. Apply them to every project created from this template unless the owner says otherwise in the conversation.

## Concurrency and Token Budget

- Run **at most 2 agents at the same time**. The owner's token quota regenerates every six hours, and more parallel agents drain it before it refills. Split larger fan-outs into pairs.
- Prefer fewer, well-briefed agents over many small ones. Point agents at the exact spec sections and files they need instead of asking them to read everything.
- When the owner asks how much budget a plan will use, estimate it from what comparable finished steps cost and say which parts are uncertain.

## Resumable Work

The owner may pause the run at any time and resume hours later.

1. Keep a roadmap file with one checkbox per step (for example `docs/design/roadmap.md`) and a "How to Resume" section.
2. Every agent commits and pushes after each finished sub-step. Commit subjects use the step id as the scope (`feat(p2.b): ...`) and a `Roadmap-Step:` trailer.
3. An agent that must stop early pushes a `wip(<step>): ...` commit that says what remains.
4. Workflows are written so a resumed run skips finished steps (finished agent calls replay from cache; finished work is already on the branch).

## Verification Strategy

Goal: catch problems early with as few tokens as possible, without lowering quality.

1. **Automated gate first (cheap).** Every step passes `make check`, its own tests, and CI on the pushed commit. Implementers write a test for every rule they implement.
2. **Self-review.** Before committing, the implementing agent checks its own diff against the project rules and the spec section it implemented. Its context is already loaded, so this costs little.
3. **Diff-scoped reviews.** Reviewers read `git diff <last-reviewed>..HEAD` and the files it touches, not the whole repository.
4. **Review depth follows risk:**

   | Risk | Examples | Review |
   | --- | --- | --- |
   | High | Authentication, sessions, secrets and encryption, tenant isolation (one user must never see another's data), outbound requests (SSRF), tool or permission approvals, schedulers, concurrency and run state, migrations (schema, foreign keys, cascades), protocol adapters | One combined reviewer plus one security reviewer, right after the phase |
   | Low | Display-only web screens, styling, docs, CI wiring that its own run proves, test additions, mechanical refactors | One combined reviewer, batched every two phases |

5. **One fixer per review round.** The fixer confirms each finding itself before changing code, fixes confirmed high and medium findings plus cheap low ones, and skips (with a reason) findings it can show are wrong. No separate "verify the finding" agent.
6. **Final gate.** The last hardening phase runs a full multi-lens review (correctness, security, layering rules, UX, footprint) over the whole codebase.

## Model Choice

- **Smaller, cheaper model** (for example the current Sonnet), only when the task is low risk, involves no security concern, and a missed issue would be cheap to fix later:
  - Diff reviews of docs, README, and changelog.
  - Diff reviews of display-only web screens (layout, styling, empty and loading states).
  - CI and workflow configuration whose own run proves it works.
  - Adding Bruno or other tests, checking generated contract drift.
  - Mechanical follow-ups: renames, lint fixes, moving files.
- **Full model** for everything else, always including the high-risk areas in the table above, all core logic, and the final review.
- When unsure, use the full model.
- Workflow summaries say which steps used the smaller model.

## GitHub Actions Runners

Use the smallest runner that finishes the job:

| Job | Runner |
| --- | --- |
| Lint, Markdown, TypeScript type-check, web build, path filters | `ubuntu-slim` (1 vCPU, 5 GB, 15-minute cap, no Docker) |
| Rust build, Clippy, tests, jobs that need a Postgres service | `ubuntu-latest` |
| Linux desktop bundles (glibc floor matters) | a pinned `ubuntu-XX.04` |
| Android builds | `ubuntu-latest` |
| macOS or iOS builds | `macos-latest`, only when needed (tags or manual dispatch) |

Check the current runner catalog before relying on these numbers; it changes.
