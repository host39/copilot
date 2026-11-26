# ADR-005: Copilot Architecture

## Status

Proposed

## Context

Copilot turns the RustInit template into a lightweight, self-hosted chat client for any model provider. The owner asked for five capabilities: project files, reusable system prompts ("agents"), chat with any model, prompts that run at a set time, and calls to external systems. The owner also set limits: no machine learning inside Copilot, a footprint much lighter than Open WebUI, voice only where a provider supports it natively, and clients for the web, Linux desktop, and Android.

A six-lens review (UX, architecture, security, protocols, footprint, delivery) of the first design draft found that the hard parts (the conversation engine, tool approvals, the run registry, the scheduler, tenant isolation, and the install) needed explicit decisions before the first migrations are written, because applied migrations are immutable. The full spec is [docs/design/copilot.md](../design/copilot.md).

## Decision

### Product Model

- **Projects replace agents.** One concept (name, instructions, files, default model, reasoning level, tools, schedules) covers both "saved prompt" and "saved prompt plus files". A chat belongs to a project or stands alone. Users also have personal instructions.
- **Four concepts:** Project, Chat, Schedule, Tool server. "Session" means a login only.

### Forwarding Only

- Copilot never runs a model: no local speech-to-text, text-to-speech, embeddings, inference, or OCR. When a model cannot read a part (an image on a text-only model, a PDF on a self-hosted model), the part is replaced by a deterministic notice instead of being converted.
- No retrieval-augmented generation in v1: project files go to the model as context, with a token estimate and a capacity bar.

### Provider Protocols

- Two wire protocols in v1: OpenAI-compatible Chat Completions (OpenAI, OpenRouter, Ollama, vLLM, LM Studio, llama.cpp, Groq, Together, Gemini) and the Anthropic Messages API. A connection has a kind and a preset; the preset fixes the request dialect.
- Gemini goes through Google's OpenAI-compatible endpoint, gated by a spike. If the spike fails, a native adapter targets the Interactions API, not the legacy `generateContent`.
- The OpenAI Responses API is a later kind; until then, models that need it for tool calling have tool use locked off.
- Messages are stored as provider-neutral parts plus a separate, verbatim provider state (thinking blocks and signatures, reasoning details). Provider state is replayed only to the same connection, for the current turn or when the turn's context fingerprint matches. Each turn's system prompt and tool list are snapshotted, so settings changes apply from the next user message and never break Anthropic's preserved-thinking prefix check.

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

- Authentication is built in: no auth bypass setting. The first registration needs a setup token; admins create other users unless registration is open.
- Tenant isolation is enforced by composite foreign keys that include `user_id`, plus user-scoped repository methods.
- Secrets are sealed with AES-256-GCM in a versioned format with a key id and row-bound associated data. `SECRET_KEY` is required everywhere.
- Outbound requests share one hardened client policy (DNS-time address checks, no redirects, no proxy, metadata addresses always blocked); private networks are reachable only by admins' connections by default.

## Consequences

### Positive

- One context concept and one tool code path for every provider.
- No in-memory state that matters across a restart: approvals, schedules, and messages are all persisted.
- A self-hoster runs one image, one Postgres volume, and two secrets.
- The interfaces that parallel work depends on are fixed before tracks start.

### Negative

- PDFs cannot be read by self-hosted models in v1.
- Gemini depends on Google's OpenAI compatibility layer until the spike proves it, or a native adapter is built.
- Large files in Postgres make the database larger and load whole files into memory (bounded by the upload limit).
- One process caps scale; serverless hosting is not possible.
- Users in a household each add their own provider keys until shared connections arrive.

### Neutral

- The OpenAI Responses API, MCP OAuth, stdio servers, live voice, and key rotation tooling are deferred, and the data model leaves room for each.

## Alternatives Considered

| Alternative | Why rejected |
| --- | --- |
| Separate "agents" concept next to projects | Two overlapping concepts for the same need |
| Local speech or OCR models for missing capabilities | Violates the no-ML constraint and the footprint goal |
| Native Gemini `generateContent` adapter in v1 | A third wire protocol on an API Google calls legacy; the compatibility endpoint may suffice |
| Anthropic's MCP connector | Anthropic only, no approval step, public HTTPS servers only |
| Holding a run in memory while waiting for approval | Lost on restart, holds the run slot for hours, breaks provider history rules |
| `FOR UPDATE SKIP LOCKED` scheduler | Needs a transaction across ports and holds pool connections during long runs |
| Multiple API instances or Lambda | The run registry, scheduler, and live streams need one long-lived process |
| SQLite by default | Template investment in Postgres, and a single writer contends with streaming saves |
| Files on disk under a data directory | A second volume to back up and no transaction across row and blob |

## References

- [Copilot design spec](../design/copilot.md)
- [Build roadmap](../design/roadmap.md)
- [ADR-001: Domain-Driven Design](./001-domain-driven-design.md)
- [ADR-002: Database Migrations and Local Seed](./002-database-migrations-and-seed.md)
- [ADR-003: Rust Backend Stack](./003-rust-backend-stack.md)
- [ADR-004: Contracts and Type Sharing](./004-contracts-and-type-sharing.md)
- Research notes: [providers](../research/providers.md), [MCP](../research/mcp.md), [voice](../research/voice.md), [native apps and CI](../research/native-and-ci.md), [dependencies](../research/dependencies.md)
