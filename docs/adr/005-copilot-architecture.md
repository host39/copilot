# ADR-005: Copilot Architecture

## Status

Proposed

## Context

Copilot turns the RustInit template into a lightweight, self-hosted chat client for any model provider. The owner asked for five capabilities: project files, reusable system prompts ("agents"), chat with any model, prompts that run at a set time, and calls to external systems. The owner also set limits: no machine learning inside Copilot, a footprint much lighter than Open WebUI, voice only where a provider supports it natively, and clients for the web, Linux desktop, and Android.

A six-lens review (UX, architecture, security, protocols, footprint, delivery) of the first design draft found that the hard parts (the conversation engine, tool approvals, the run registry, the scheduler, tenant isolation, and the install) needed explicit decisions before the first migrations are written, because migrations become immutable once released or deployed. The owner later decided that identity is shared across the owner's apps through lib-rustauth instead of being built in Copilot. The full spec is [docs/design/copilot.md](../design/copilot.md).

## Decision

### Product Model

- **Projects replace agents.** One concept (name, instructions, files, default model, reasoning level, tools, schedules) covers both "saved prompt" and "saved prompt plus files". A chat belongs to a project or stands alone. Users also have personal instructions.
- **Four concepts:** Project, Chat, Schedule, Tool server. "Session" means a login only.

### Identity Comes From lib-rustauth

- Copilot does not build users, passwords, sessions, roles, user administration, login throttling, or the setup token. It uses lib-rustauth (`host39/lib-rustauth`), the owner's shared authentication library, as a Cargo git dependency pinned by commit until `v0.1.0` is tagged.
- rustauth's router is mounted under `/api` (`/auth/*`, `/admin/users*`), and its guard protects every other `/api` route. Its migrations create the `auth` schema and run before Copilot's; Copilot tables reference `auth.users (id)` with `ON DELETE CASCADE`.
- `copilot-core` never depends on a rustauth crate. It has its own `UserId` and an `Actor { user_id, is_admin }` value that `apps/api` builds from rustauth's `AuthUser` (`is_admin` means the role `administrator`). `apps/api` maps `AuthError` into `DomainError`.
- Personal instructions are Copilot data, in a Copilot table `user_settings`.

**Why:** every future app of the owner gets authentication and authorization by adding the library, so security fixes land once, and SSO, passkeys, and two-factor sign-in are added later in one place instead of in each app. rustauth uses the same clean/DDD layering and crate versions as the RustInit template, and the same error envelope, so Copilot keeps one copy of each crate and one error shape. Keeping rustauth in the same database (its own schema) keeps foreign keys and cascades, so tenant isolation and user deletion stay database-enforced.

### Forwarding Only

- Copilot never runs a model: no local speech-to-text, text-to-speech, embeddings, inference, or OCR. When a model cannot read a part (an image on a text-only model, a PDF on a self-hosted model), the part is replaced by a deterministic notice instead of being converted.
- No retrieval-augmented generation in v1: project files go to the model as context, with a token estimate and a capacity bar.

### Provider Protocols

- Two wire protocols in v1: OpenAI-compatible Chat Completions (OpenAI, OpenRouter, Ollama, vLLM, LM Studio, llama.cpp, Groq, Together, Gemini) and the Anthropic Messages API. A connection has a kind and a preset; the preset fixes the request dialect.
- Gemini goes through Google's OpenAI-compatible endpoint, gated by a spike. If the spike fails, a native adapter targets the Interactions API, not the legacy `generateContent`.
- The OpenAI Responses API is a later kind; until then, models that need it for tool calling have tool use locked off.
- Messages are stored as provider-neutral parts plus a separate, verbatim provider state (thinking blocks and signatures, reasoning details). Provider state is replayed only to the same connection, for the current turn or when the turn's context fingerprint matches. Each turn's system prompt and tool list are snapshotted, so settings changes apply from the next user message and never break Anthropic's preserved-thinking prefix check.

### Gemini Through the OpenAI-Compatible Endpoint

Spike outcome (roadmap P2.B.5, 2026-11-27): **no native Gemini adapter in v1.** The `gemini` preset stays on Google's OpenAI-compatible endpoint.

- **What the spike checked.** No Gemini key was available, so the spike ran on wiremock fixtures shaped after Google's documentation (`packages/providers/tests/openai.rs`, the Gemini spike section): a streamed tool call over two turns with its thought signature replayed; parallel calls where only the first call of the step is signed; sequential steps where every signature of the current turn goes back; results sent after the model turn in call order, never interleaved; the system and tool roles; images as base64 data URLs; `input_audio` WAV; `reasoning_effort`.
- **Why it passes.** The adapter keeps every extra field of a tool call on the `gemini` dialect (Google's pages do not show the field path inside `tool_calls`; the fixtures use `extra_content.google.thought_signature`), so the round trip does not depend on one path. Core replays that state only to the same model on the same connection. A step that carries no signature (an answer rebuilt from its parts, for example after a model switch) gets Google's documented stand-in, `skip_thought_signature_validator`, on its first call, so foreign history does not fail the signature check.
- **What stays open.** Google calls its OpenAI compatibility "still in beta". PDF `file` parts are not documented there (core keeps `pdf` off for the `gemini` preset unless the user turns it on). Thought summaries are not requested: Google documents them only through `extra_body.google.thinking_config.include_thoughts`, which cannot be combined with `reasoning_effort`, so Gemini reasoning text is not shown in v1. Gemini 3 and 4 cannot turn reasoning off, so the `off` level works only on 2.5 models. A call without an `id` cannot carry its signature, because replay state is keyed by call id.
- **Live check.** The P3.4 owner checkpoint runs one Gemini 3 or 4 tool loop over two turns. If it fails the signature check (400 "missing a `thought_signature`"), or if thought summaries become a requirement, the native adapter targets the Interactions API with `store: false`, not the legacy `generateContent`.

### Tools Through MCP

- Tools come from remote MCP servers over Streamable HTTP through the official `rmcp` client, dual-era (2026-07-28 and 2025 servers). Copilot runs every tool call itself, for every provider; it does not use provider-hosted MCP connectors.
- stdio servers and MCP OAuth are out of v1. A stdio server runs in its own container behind a bridge and is registered by URL.
- New servers default to `ask`; modes can be set per tool. Approvals are **persisted** on the message (`awaiting_approval`) and the run ends, so any device can answer and a restart loses nothing.

### Detached Runs With a Run Registry

- A turn runs in a task that outlives the HTTP request. A per-process run registry gives single-flight per chat, cancellation, reattach with sequence numbers, and a per-user event stream.
- **Exactly one API process.** The `lambda` entrypoint is removed. Shutdown cancels runs cooperatively so partial output is saved; startup marks interrupted messages as errors.

### System-Clock Scheduler

- An in-process loop fires due schedules. The model never decides when a run happens. Each schedule is claimed with a compare-and-set update and no row lock is held during the run. Missed fires collapse into one run. Each run creates a new chat.
- The model may create schedules only through built-in tools that always ask for approval and are never offered to unattended runs.

### Voice Only Where Native

- A microphone appears only for models that accept audio input, and spoken replies only for models that return audio. Claude models get no voice controls. Speech and transcription endpoints, live voice, and voice for text-only models are later work and need the owner's opt-in.

### Native Shells

- The Linux and Android apps are Tauri v2 shells around the same web bundle. They call the API through `tauri-plugin-http`, which avoids CORS, mixed content, and Android cleartext limits. The web bundle is also installable as a web app.
- `apps/web` may depend on `@tauri-apps/api`, `@tauri-apps/plugin-http`, `react-markdown`, `remark-gfm`, and `@tanstack/react-query`, an exception to the rule that it imports only `@copilot/contracts`.

### Install and Storage

- **Postgres is kept** for v1: the template's database layer, readonly role, tests, and CI are built on it, and streaming writes plus the scheduler need concurrent writers. The schema stays portable (ids from Rust, no enum types, no triggers or `LISTEN/NOTIFY`) so a SQLite adapter can be added later behind the same ports.
- **File bytes live in Postgres**, so one `pg_dump` is the whole backup and deletes are transactional.
- One image holds the `server` and `migrate` binaries and the web bundle. The API serves the SPA from the same origin and lives under `/api`. A compose file runs Postgres, a one-off migrate service, and the server.

### Security Baseline

- Authentication is always on and comes from lib-rustauth: no auth bypass setting. The first registration needs a setup token; administrators create other users unless registration is open.
- Tenant isolation is enforced by composite foreign keys that include `user_id`, plus user-scoped repository methods.
- Secrets are sealed with AES-256-GCM in a versioned format with a key id and row-bound associated data. `SECRET_KEY` is required everywhere.
- Outbound requests share one hardened client policy (DNS-time address checks, no redirects, no proxy, metadata addresses always blocked); private networks are reachable only by admins' connections by default.

## Consequences

### Positive

- One context concept and one tool code path for every provider.
- No in-memory state that matters across a restart: approvals, schedules, and messages are all persisted.
- A self-hoster runs one image, one Postgres volume, and two secrets.
- The interfaces that parallel work depends on are fixed before tracks start.
- Identity is shared with the owner's other apps: one implementation to review and test, and later sign-in methods arrive through a version bump.

### Negative

- PDFs cannot be read by self-hosted models in v1.
- Gemini depends on Google's OpenAI compatibility layer, which Google calls beta: no thought summaries in v1, PDFs undocumented there, and the signature round trip proven on fixtures until the P3.4 live check.
- Large files in Postgres make the database larger and load whole files into memory (bounded by the upload limit).
- One process caps scale; serverless hosting is not possible.
- Users in a household each add their own provider keys until shared connections arrive.
- Copilot depends on a second repository: a change to identity is made in lib-rustauth first, then Copilot bumps its pinned commit or tag. Until lib-rustauth's steps land, the Copilot steps that need them wait (see the roadmap's Track R).
- Two migration histories share one database (`auth._sqlx_migrations` and Copilot's). Copilot's scheduler resolves schedule owners through rustauth's `AuthService::users` (core's `Accounts` port), never by reading `auth.users` beyond its `id`.

### Neutral

- The OpenAI Responses API, MCP OAuth, stdio servers, live voice, and key rotation tooling are deferred, and the data model leaves room for each.

## Alternatives Considered

| Alternative | Why rejected |
| --- | --- |
| Separate "agents" concept next to projects | Two overlapping concepts for the same need |
| Identity built inside Copilot (spec v2) | Every later app would copy it, and SSO, passkeys, and two-factor sign-in would be built once per app |
| A separate authentication service with JWTs | A second process to deploy, against the one-process constraint; opaque sessions are revocable at once. rustauth can add a token issuer later if other services need tokens |
| rustauth in a separate database | No foreign keys or cascades across databases, so user deletion and tenant checks would need application code |
| Local speech or OCR models for missing capabilities | Violates the no-ML constraint and the footprint goal |
| Native Gemini `generateContent` adapter in v1 | A third wire protocol on an API Google calls legacy; the compatibility endpoint may suffice |
| Native Gemini Interactions adapter after the spike | The spike's fixtures pass on the compatibility endpoint; a third wire protocol waits until a live check fails or thought summaries are required |
| Anthropic's MCP connector | Anthropic only, no approval step, public HTTPS servers only |
| Holding a run in memory while waiting for approval | Lost on restart, holds the run slot for hours, breaks provider history rules |
| `FOR UPDATE SKIP LOCKED` scheduler | Needs a transaction across ports and holds pool connections during long runs |
| Multiple API instances or Lambda | The run registry, scheduler, and live streams need one long-lived process |
| SQLite by default | Template investment in Postgres, and a single writer contends with streaming saves |
| Files on disk under a data directory | A second volume to back up and no transaction across row and blob |

## References

- [Copilot design spec](../design/copilot.md)
- [Build roadmap](../design/roadmap.md)
- lib-rustauth design spec: `docs/design/rustauth.md` in `host39/lib-rustauth`
- [ADR-001: Domain-Driven Design](./001-domain-driven-design.md)
- [ADR-002: Database Migrations and Local Seed](./002-database-migrations-and-seed.md)
- [ADR-003: Rust Backend Stack](./003-rust-backend-stack.md)
- [ADR-004: Contracts and Type Sharing](./004-contracts-and-type-sharing.md)
- Research notes: [providers](../research/providers.md), [MCP](../research/mcp.md), [voice](../research/voice.md), [native apps and CI](../research/native-and-ci.md), [dependencies](../research/dependencies.md)
