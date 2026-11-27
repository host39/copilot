# ADR-005: Copilot Architecture

## Status

Accepted (2026-11-27): built as decided through roadmap Phase 6, with the changes recorded below (Gemini, the fake model server, voice, and the native shells), and amended for the Phase 7 steps (the OpenAI Responses API in P7.1, MCP OAuth sign-in in P7.2, built-in stdio servers in P7.3). The owner's review of the spec and this ADR (roadmap P0.3) is still open; anything it changes comes as an amendment here or a new ADR.

## Context

Copilot turns the RustInit template into a lightweight, self-hosted chat client for any model provider. The owner asked for five capabilities: project files, reusable system prompts ("agents"), chat with any model, prompts that run at a set time, and calls to external systems. The owner also set limits: no machine learning inside Copilot, a footprint much lighter than Open WebUI, voice only where a provider supports it natively, and clients for the web, Linux desktop, and Android.

A six-lens review (UX, architecture, security, protocols, footprint, delivery) of the first design draft found that the hard parts (the conversation engine, tool approvals, the run registry, the scheduler, tenant isolation, and the install) needed explicit decisions before the first migrations are written, because migrations become immutable once released or deployed. The owner later decided that identity is shared across the owner's apps through lib-rustauth instead of being built in Copilot. The full spec is [docs/design/copilot.md](../design/copilot.md).

## Decision

### Product Model

- **Projects replace agents.** One concept (name, instructions, files, default model, reasoning level, tools, schedules) covers both "saved prompt" and "saved prompt plus files". A chat belongs to a project or stands alone. Users also have personal instructions.
- **Four concepts:** Project, Chat, Schedule, Tool server. "Session" means a login only.

### Identity Comes From lib-rustauth

- Copilot does not build users, passwords, sessions, roles, user administration, login throttling, or the setup token. It uses lib-rustauth (`host39/lib-rustauth`), the owner's shared authentication library, as a Cargo git dependency pinned by commit: the `v0.1.0` release commit, until that tag is pushed and the pin becomes `tag = "v0.1.0"`.
- rustauth's router is mounted under `/api` (`/auth/*`, `/admin/users*`), and its guard protects every other `/api` route. Its migrations create the `auth` schema and run before Copilot's; Copilot tables reference `auth.users (id)` with `ON DELETE CASCADE`.
- `copilot-core` never depends on a rustauth crate. It has its own `UserId` and an `Actor { user_id, is_admin }` value that `apps/api` builds from rustauth's `AuthUser` (`is_admin` means the role `administrator`). `apps/api` maps `AuthError` into `DomainError`.
- Personal instructions are Copilot data, in a Copilot table `user_settings`.

**Why:** every future app of the owner gets authentication and authorization by adding the library, so security fixes land once, and SSO, passkeys, and two-factor sign-in are added later in one place instead of in each app. rustauth uses the same clean/DDD layering and crate versions as the RustInit template, and the same error envelope, so Copilot keeps one copy of each crate and one error shape. Keeping rustauth in the same database (its own schema) keeps foreign keys and cascades, so tenant isolation and user deletion stay database-enforced.

### Forwarding Only

- Copilot never runs a model: no local speech-to-text, text-to-speech, embeddings, inference, or OCR. When a model cannot read a part (an image on a text-only model, a PDF on a self-hosted model), the part is replaced by a deterministic notice instead of being converted.
- No retrieval-augmented generation in v1: project files go to the model as context, with a token estimate and a capacity bar.

### Provider Protocols

- Two wire protocols in v1: OpenAI-compatible Chat Completions (OpenAI, OpenRouter, Ollama, vLLM, LM Studio, llama.cpp, Groq, Together, Gemini) and the Anthropic Messages API; P7.1 added OpenAI's Responses API as a third. A connection has a kind and a preset; the preset fixes the request dialect.
- Gemini goes through Google's OpenAI-compatible endpoint (the spike's outcome, below). A native adapter, if a live check ever needs one, targets the Interactions API, not the legacy `generateContent`.
- The OpenAI Responses API (P7.1) is not a third kind but a per-connection choice of `openai` connections (`providers.api`): the `openai` preset uses Responses by default, the `custom` preset Chat Completions with an opt-in, and the `gemini`, `openrouter`, and `ollama` presets always Chat Completions (see "OpenAI Responses API" below).
- Messages are stored as provider-neutral parts plus a separate, verbatim provider state (thinking blocks and signatures, reasoning details). Provider state is replayed only to the same connection, for the current turn or when the turn's context fingerprint matches. Each turn's system prompt and tool list are snapshotted, so settings changes apply from the next user message and never break Anthropic's preserved-thinking prefix check.

### Gemini Through the OpenAI-Compatible Endpoint

Spike outcome (roadmap P2.B.5, 2026-11-27): **no native Gemini adapter in v1.** The `gemini` preset stays on Google's OpenAI-compatible endpoint.

- **What the spike checked.** No Gemini key was available, so the spike ran on wiremock fixtures shaped after Google's documentation (`packages/providers/tests/openai.rs`, the Gemini spike section): a streamed tool call over two turns with its thought signature replayed; parallel calls where only the first call of the step is signed; sequential steps where every signature of the current turn goes back; results sent after the model turn in call order, never interleaved; the system and tool roles; images as base64 data URLs; `input_audio` WAV; `reasoning_effort`.
- **Why it passes.** The adapter keeps every extra field of a tool call on the `gemini` dialect (Google's pages do not show the field path inside `tool_calls`; the fixtures use `extra_content.google.thought_signature`), so the round trip does not depend on one path. Core replays that state only to the same model on the same connection. A step that carries no signature (an answer rebuilt from its parts, for example after a model switch) gets Google's documented stand-in, `skip_thought_signature_validator`, on its first call, so foreign history does not fail the signature check.
- **What stays open.** Google calls its OpenAI compatibility "still in beta". PDF `file` parts are not documented there; the `gemini` preset nevertheless starts with `vision`, `pdf`, and `reasoning` on (changed in P6.1: the listing reports no metadata, and every current Gemini model reads images and PDFs and thinks), and the P3.4 live check should confirm the PDF part along with the tool loop. The compatibility listing names models only, so since P6.4 the refresh also reads Google's native model list for each model's context window, output limit, and thinking flag (a failure keeps the listing as it was). Audio input goes as `input_audio` (P4.1); audio output stays off by default because the endpoint documents none, but it is not locked. Thought summaries are not requested: Google documents them only through `extra_body.google.thinking_config.include_thoughts`, which cannot be combined with `reasoning_effort`, so Gemini reasoning text is not shown in v1. Gemini 3 and 4 cannot turn reasoning off, so the `off` level works only on 2.5 models. A call without an `id` cannot carry its signature, because replay state is keyed by call id.
- **Live check.** The P3.4 owner checkpoint runs one Gemini 3 or 4 tool loop over two turns. If it fails the signature check (400 "missing a `thought_signature`"), or if thought summaries become a requirement, the native adapter targets the Interactions API with `store: false`, not the legacy `generateContent`.

### OpenAI Responses API

Added in P7.1 (2026-11-27), after v0.1, so OpenAI's newest models get tool calling (GPT-6 Astra and GPT-6.1 Sol call tools only through Responses), reasoning with tools (GPT-5.4 and later refuse tools on Chat Completions unless reasoning is off), and reasoning summaries (a Responses-only feature). Shapes were checked against OpenAI's API reference and streaming event reference on 2026-11-27 (`docs/research/providers.md`, OpenAI Responses).

- **A per-connection API, not a kind.** Both APIs share the base URL, the bearer key, and `GET {base}/models`, so the kind stays the wire family and `providers.api` (`chat_completions` or `responses`, null for `anthropic` connections, enforced by a `CHECK`) picks the endpoint of the answers. `OpenAiApi::fits` in core holds the preset rule. A third kind would have doubled the preset table and the model listing for no gain.
- **Routing.** The `openai` preset defaults to Responses; it may switch to Chat Completions because audio (`input_audio`, spoken replies) is still Chat Completions only, and the Responses API locks `audio_in` and `audio_out`. The `custom` preset defaults to Chat Completions, the endpoint every compatible server serves, and may opt in. `gemini`, `openrouter`, and `ollama` stay on Chat Completions (no documented Responses endpoint on Google's compatibility layer; dialects tuned to Chat Completions). The migration moved existing `openai` preset connections to Responses and restored the `tools` flag of their `gpt-6*` models, which only the old lock had turned off; refreshes now store detected capabilities before locks, so an API switch moves its locks at once.
- **Stateless.** Requests send `store: false` and the whole history every time, as Chat Completions does; nothing is kept on OpenAI's side, so `previous_response_id` is never used. The answer's finished output items (encrypted reasoning, messages with their `phase`, function calls) are its provider state, replayed under the existing replay rules (same connection; the current turn or the same context hash). A reasoning item without `encrypted_content` cannot be replayed statelessly and is dropped together with the other items' ids. State written by the other API reads as absent, so a switch rebuilds earlier answers from their parts.
- **Why:** this unlocks the models the owner is most likely to add first on an OpenAI key, with no new dependency and no change to the core port beyond `ProviderTarget::api`. The fake model server speaks Responses too (and refuses what the real API refuses statelessly, such as reasoning without its encrypted content), and the smoke and browser tests run on it.

### Tools Through MCP

- Tools come from remote MCP servers over Streamable HTTP through the official `rmcp` client, dual-era (2026-07-28 and 2025 servers). Copilot runs every tool call itself, for every provider; it does not use provider-hosted MCP connectors.
- stdio servers were out of v1 (a stdio server ran in its own container behind a bridge, registered by URL, which still works); built-in stdio servers for administrators came in P7.3 (below). MCP OAuth was out of v1 and came in P7.2 (below).
- New servers default to `ask`; modes can be set per tool. Approvals are **persisted** on the message (`awaiting_approval`) and the run ends, so any device can answer and a restart loses nothing.

### MCP OAuth Sign-In

Added in P7.2 (2026-11-27), after v0.1, for remote MCP servers that require OAuth, per the MCP authorization specification, revision `2026-07-28` (checked on modelcontextprotocol.io the same day). The flow, the checks, and the residual risk are in the spec's "Tool Server Sign-In"; the decisions with their reasons are in [tracks/p7-2.md](../design/tracks/p7-2.md).

- **A per-server auth mode.** `tool_servers.auth_mode` is `none`, `headers`, or `oauth`, and each mode's credentials exist only in that mode (a `CHECK` holds it), so a token never lingers after a switch.
- **The flow lives server side.** The redirect URI is the API (`<PUBLIC_URL>/api/oauth/callback`), not the device, so the web app, the Linux app, and Android all open one URL in a browser and tokens never reach a client. The PKCE verifier and the recorded issuer wait sealed in `oauth_flows` (the `state` only as its SHA-256) for 10 minutes and are used once. Tokens are sealed into the server's row together with everything a refresh needs.
- **Hand-written OAuth requests, not rmcp's `auth` feature.** rmcp's `AuthorizationManager` owns the flow state and credentials in its own stores and takes one HTTP client; Copilot's flow spans two requests and a restart, keeps state in Postgres sealed per row, and picks a hardened client per URL and network scope for every request (the SSRF rules and `ALLOW_PRIVATE_NETWORK` apply to every URL a server hands back). The feature would also have added 14 crates to the server's graph, among them second copies of `base64`, `rand`, `sha2`, and `thiserror`. The four OAuth requests sit in `copilot-mcp` behind core's `ToolAuth` port; no `aws-lc-rs`.
- **Refresh in core, used by the adapter.** `ToolEndpoint::tokens` hands the MCP adapter an `AccessTokens` source: core refreshes an expiring token first, and the adapter asks for a fresh one once after a `401` and reconnects. A refresh the authorization server rejects marks the server as needing sign-in; a network failure does not.
- **`PUBLIC_URL`** (https, or http on loopback) turns sign-in on; `OAUTH_CLIENT_METADATA` lets an https install offer its Client ID Metadata Document, which the specification prefers to dynamic registration.
- **Why:** servers that only offer OAuth (many hosted MCP servers) become usable without static tokens, with no client-side token storage and no new dependency.
- **Accepted risk:** the callback is public, so a user of the same instance could have another user complete a flow the first one started; see the spec's residual risk.

### Detached Runs With a Run Registry

- A turn runs in a task that outlives the HTTP request. A per-process run registry gives single-flight per chat, cancellation, reattach with sequence numbers, and a per-user event stream.
- **Exactly one API process.** The `lambda` entrypoint is removed. Shutdown cancels runs cooperatively so partial output is saved; startup marks interrupted messages as errors.

### System-Clock Scheduler

- An in-process loop fires due schedules. The model never decides when a run happens. Each schedule is claimed with a compare-and-set update and no row lock is held during the run. Missed fires collapse into one run. Each run creates a new chat.
- The model may create schedules only through built-in tools that always ask for approval and are never offered to unattended runs.

### Voice Only Where Native

- A microphone appears only for models that accept audio input, and spoken replies only for models that return audio. Claude models get no voice controls (their audio flags are locked off). Speech and transcription endpoints, live voice, and voice for text-only models are later work and need the owner's opt-in.
- **As built (P4.1, P4.2).** The browser records through an AudioWorklet and encodes 16 kHz mono PCM16 WAV in TypeScript (no codec library, no server-side transcoding); a clip stops at 5 minutes and is uploaded as a chat file, so it is previewed, counted against the storage quota, and sent as an `input_audio` part. Spoken replies stream as `audio.delta` events and play through Web Audio; when the answer ends, the server saves the streamed PCM16 as a 24 kHz WAV chat file on every exit and keeps the transcript as the message text, and later turns send only the transcript. Audio a model cannot take is replaced by a notice, like any other unsupported part.
- **Why:** WAV needs no encoder dependency in the browser or the server and every audio-capable chat API accepts it; saving the reply as a file reuses the file limits, downloads, and deletion rules instead of a second storage path.

### Native Shells

- The Linux and Android apps are Tauri v2 shells around the same web bundle. They call the API through `tauri-plugin-http`, which avoids CORS, mixed content, and Android cleartext limits. The web bundle is also installable as a web app.
- `apps/web` may depend on `@tauri-apps/api`, `@tauri-apps/plugin-http`, `react-markdown`, `remark-gfm`, and `@tanstack/react-query`, an exception to the rule that it imports only `@copilot/contracts`.
- **As built (P5.1 to P5.3, P6.1).** The shell lives in `apps/native`, outside the Cargo workspace, so `make check` never needs WebKitGTK. The server URL is entered on a Connect screen and stored on the device (plain `http` to a public host needs consent). The capability grants the HTTP plugin, the opener for `http`, `https`, and `mailto` links, and one app command, `save_file`, which saves downloads through the system's save dialog (the dialog and file system plugins expose no command to the web view). A navigation guard keeps the web view on the bundle's origin and opens other links in the system browser, and the web view's permission requests are answered with the microphone only. CI builds the `.deb`, the AppImage (pinned Ubuntu 24.04, glibc 2.39), and an arm64 APK as workflow artifacts (and, on tags, release files); tags are release-signed with one keystore from repository secrets, other builds with the runner's debug key. There is no app store distribution in v1.
- **Why:** one interface to build and test, the smallest native surface that covers downloads and the microphone, and no Tauri code in `make check`.

### Install and Storage

- **Postgres is kept** for v1: the template's database layer, readonly role, tests, and CI are built on it, and streaming writes plus the scheduler need concurrent writers. The schema stays portable (ids from Rust, no enum types, no triggers or `LISTEN/NOTIFY`) so a SQLite adapter can be added later behind the same ports.
- **File bytes live in Postgres**, so one `pg_dump` is the whole backup and deletes are transactional.
- One image holds the `server` and `migrate` binaries and the web bundle. The API serves the SPA from the same origin and lives under `/api`. A compose file runs Postgres, a one-off migrate service, and the server.
- **As built (P1.7, P6.1).** The image is distroless (`cc-debian12`, `nonroot`) with binaries from a `dist` profile (fat LTO, one codegen unit, stripped) and a precompressed bundle. `migrate` connects as the database owner and creates the server's role `copilot_server`, which may change rows but no schema, so the running server never holds the owner's password. Measured sizes are in [docs/footprint.md](../footprint.md).
- **Releases (P6.4).** A `v*` tag publishes the image the Image job tested to `ghcr.io/host39/copilot` (amd64; `<version>`, `<major>.<minor>`, and `latest`) and attaches the `.deb`, AppImage, and APK with `SHA256SUMS` to the tag's GitHub Release, with that version's `CHANGELOG.md` section as notes; installs set `COPILOT_IMAGE` and build nothing. CI also runs `migrate` twice, restores a backup, and upgrades a database written by the previous tag. There is no auto-update in v1: an updater needs a signing key and a feed, a later decision.

### Fake Model Server for Development and CI

- `tools/fake-llm` (added in P4.3 at the owner's request) is a dev-only workspace binary that speaks every wire protocol Copilot uses (OpenAI's Chat Completions and, since P7.1, Responses, and Anthropic's Messages, streamed or whole) and serves an MCP server at `/mcp` (both eras). Keywords in the newest user message choose its behavior: errors with provider-shaped bodies, a broken stream, reasoning, tool calls, refusals, and audio. `make smoke` and CI drive the real API over HTTP against it (both provider kinds, a streamed reply, a provider error, an approved MCP tool call), and `make fake-llm` lets a person click through every chat path without a key.
- No Copilot crate depends on it and it depends on none (`make boundaries`), so it never reaches the image.
- **Why:** wiremock fixtures test each adapter in isolation, but only a whole server exercises the API, the run registry, the outbound client policy, and the web app together; a fake that follows the real APIs' rules (for example, refusing a tool call that is not answered right away) catches what fixtures shaped by the same author miss. It does not replace live checks with real keys (roadmap P3.4).

### Built-In stdio Servers

Added in P7.3 (2026-11-27), after v0.1, under the hardening rules the spec carried since v1. The rules are in the spec's "stdio Servers"; the decisions with their reasons are in [tracks/p7-3.md](../design/tracks/p7-3.md).

- **Off by default, administrators only, checked at every start.** `MCP_STDIO_ENABLED` (default `false`) and the administrator role gate adding and changing a program, and core hands the adapter a `SpawnGate` with each stdio endpoint that reads both again (the owner's account as stored now) right before every process starts, so a demotion takes effect within a running chat.
- **No shell, no inherited environment.** The program is an absolute path run with its argument list, `env_clear()`, and only the variables the administrator entered (sealed like every other credential), in its own directory under `MCP_STDIO_DIR`. The process leads its own process group; closing the run's session stops it (stdin, `SIGTERM`, `SIGKILL`) and kills what is left of the group, and dropping the session kills the group at once.
- **rmcp's child-process transport with `process-wrap`, process owned by Copilot.** The `transport-child-process` feature adds two crates (`process-wrap` and `nix`), no `aws-lc-rs`. Copilot spawns the process itself and hands rmcp its stdout and stdin, because rmcp's `TokioChildProcess` forgets the process group when the program exits on its own, which left the program's children running.
- **The transport is fixed per server.** A stdio server has no URL, headers, or OAuth; the database `CHECK` keeps the two transports' columns apart. A changed program is reviewed again, as a changed URL is.
- **Why:** many MCP servers only speak stdio, and the bridge container is extra setup; administrators who trust a program can now run it directly. The bridge stays the recommended way for programs that should not share the server's user and files.
- **Accepted risk:** the program runs as the API's OS user, next to its files. The deployment limits it (a low-privilege user or container, `pids_limit`, memory and CPU limits); the default image ships no runtimes for it.

### Security Baseline

- Authentication is always on and comes from lib-rustauth: no auth bypass setting. The first registration needs a setup token; administrators create other users unless registration is open.
- Tenant isolation is enforced by composite foreign keys that include `user_id`, plus user-scoped repository methods.
- Secrets are sealed with AES-256-GCM in a versioned format with a key id and row-bound associated data. `SECRET_KEY` is required everywhere.
- Outbound requests share one hardened client policy (DNS-time address checks, no redirects, no proxy, metadata addresses always blocked); private networks are reachable only by admins' connections by default.

## Consequences

### Positive

- One context concept and one tool code path for every provider.
- No in-memory state that matters across a restart: approvals, schedules, and messages are all persisted.
- A self-hoster runs one image, one Postgres volume, and three secrets (`SECRET_KEY`, the database owner's password, and the server role's).
- The interfaces that parallel work depends on are fixed before tracks start.
- Identity is shared with the owner's other apps: one implementation to review and test, and later sign-in methods arrive through a version bump.

### Negative

- PDFs cannot be read by self-hosted models in v1.
- Gemini depends on Google's OpenAI compatibility layer, which Google calls beta: no thought summaries in v1, PDFs undocumented there, and the signature round trip and the PDF and audio parts proven on fixtures until the P3.4 live check.
- Large files in Postgres make the database larger, and an upload is held in memory while it is checked and stored (bounded by the upload limit and the upload slots); downloads stream in 512 KiB pieces.
- One process caps scale; serverless hosting is not possible.
- Users in a household each add their own provider keys until shared connections arrive.
- Copilot depends on a second repository: a change to identity is made in lib-rustauth first, then Copilot bumps its pinned commit or tag. Until lib-rustauth's steps land, the Copilot steps that need them wait (see the roadmap's Track R).
- Two migration histories share one database (`auth._sqlx_migrations` and Copilot's). Copilot's scheduler resolves schedule owners through rustauth's `AuthService::users` (core's `Accounts` port), never by reading `auth.users` beyond its `id`.

### Neutral

- Live voice and key rotation tooling are deferred, and the data model leaves room for each. The OpenAI Responses API came in P7.1, MCP OAuth sign-in in P7.2, and built-in stdio servers in P7.3.

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
