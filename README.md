<!-- markdownlint-disable MD033 MD041 -->

<div align="center">

# Copilot

**A lightweight, self-hosted chat app for the models you already pay for or run yourself.**

[![Rust](https://img.shields.io/badge/Rust-2024-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![React](https://img.shields.io/badge/React-19-61DAFB?logo=react&logoColor=black)](https://react.dev/)
[![Postgres](https://img.shields.io/badge/Postgres-18-336791?logo=postgresql&logoColor=white)](https://www.postgresql.org/)
[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)](https://tauri.app/)
[![License](https://img.shields.io/badge/license-PolyForm%20Noncommercial%201.0.0-lightgrey)](./LICENSE)

</div>

Copilot is a ChatGPT-style app you run on your own server. It talks to OpenAI-compatible, Anthropic, Gemini, and Ollama-type models, keeps your chats, projects, and files in your own Postgres database, and works in the browser and as Linux and Android apps.

It is deliberately small: the server only **forwards** requests to model providers and never runs a model, an embedding, or speech processing itself. One Rust binary serves the API and the web app, Postgres holds everything else, and the server idles at about 20 MB of memory ([footprint](./docs/footprint.md)). There is no Python, no GPU, and no plugin runtime.

## Features

- **Any provider.** OpenAI, Anthropic, Gemini (through Google's OpenAI-compatible endpoint), OpenRouter, Ollama, and any OpenAI-compatible server (vLLM, LM Studio, llama.cpp, Groq, gateways) or Anthropic-compatible gateway. Each person adds their own connections; API keys are sealed at rest with AES-256-GCM.
- **Chats that keep going.** Streamed answers with reasoning, stop, regenerate, retry, edit and resend, and Continue. An answer keeps streaming when you close the tab, and any of your devices can open it mid-stream. Search across every chat, unread dots, and Markdown with code copy.
- **Projects instead of agents.** A project is a reusable context: instructions (the system prompt), files, a default model and reasoning level, tools, and schedules. Every chat in it inherits them. A prompt-only "agent" is a project without files; Save as project turns a good chat into one.
- **Files.** Images, PDFs, text and code, and audio, attached to a chat or shared by a project, sent to the models that can read them (with a capacity bar against the model's context window).
- **MCP tools.** Remote MCP servers over Streamable HTTP, with Off, Ask, or Auto per tool, approval cards (Approve, Approve for this chat, Deny with a reason), and a fresh review whenever a server changes its tools.
- **Schedules.** Run a prompt on a cron rule in your time zone; each run opens a new chat. The assistant can propose a schedule with the built-in schedule tools, but only you approve it, and the system clock (never the model) decides when it runs.
- **Voice where the model has it.** Push to talk for models that take audio (OpenAI's audio models, Gemini, OpenRouter models that list audio input, compatible servers), and spoken replies for models that return audio. Claude models have no audio API, so they get no voice controls; nothing is transcribed or synthesized on the side.
- **Accounts.** The first account is the administrator; registration is closed by default and the administrator adds people. Sessions, lockout after wrong passwords, and per-address limits come from [lib-rustauth](#identity-lib-rustauth). Nobody sees anyone else's chats, files, keys, or tools.
- **Apps.** An installable web app, a Linux app (`.deb` and AppImage), and an Android app (APK), all the same interface.

Not in this version: retrieval with vector search, running code inside Copilot, stdio MCP servers (run one [behind a bridge](./docs/self-hosting.md#local-stdio-mcp-servers) instead), MCP OAuth, branching conversations, live voice, and shared provider connections ([non-goals](./docs/design/copilot.md#non-goals-for-v1)).

## Quick Start

You need Docker with Compose v2. This checks out a release (see [Releases](https://github.com/host39/copilot/releases); the commands use `v0.1.0`), pulls its published image, starts Postgres, applies the migrations, and starts the server on `127.0.0.1:3000`:

```bash
git clone --branch v0.1.0 https://github.com/host39/copilot.git
cd copilot

umask 077
cat >deploy/.env <<EOF
COPILOT_IMAGE=ghcr.io/host39/copilot:0.1.0
SECRET_KEY=$(openssl rand -base64 32)
DB_PASSWORD=$(openssl rand -hex 24)
DB_SERVER_PASSWORD=$(openssl rand -hex 24)
EOF

docker compose -f deploy/compose.yml pull
docker compose -f deploy/compose.yml up -d --no-build
```

Keep `deploy/.env`: `SECRET_KEY` seals the stored API keys, and a different one makes them unreadable. To build the image from source instead (a few minutes once), leave `COPILOT_IMAGE` out and run `docker compose -f deploy/compose.yml up -d --build`.

### First Run

The first account needs the setup token, which the server logs while no account exists (a new one at each start; or set your own with `AUTH_SETUP_TOKEN` in `deploy/.env`):

```bash
docker compose -f deploy/compose.yml logs copilot | grep rustauth.setup.token-issued
```

Open <http://127.0.0.1:3000>, create the administrator account with that token, and follow the setup: pick a provider, paste a key (or enter an Ollama address), Test, and start chatting. Add more people in Settings, Users, or set `AUTH_REGISTRATION_OPEN=true` to let them sign up.

### Reverse Proxy

The server listens on the host's loopback only. For other devices, put a reverse proxy with TLS in front; the microphone also needs `https`. With [Caddy](https://caddyserver.com/), which gets the certificate itself, this `Caddyfile` is enough:

```caddyfile
copilot.example.com {
    request_body {
        max_size 26MB
    }
    reverse_proxy 127.0.0.1:3000 {
        flush_interval -1
    }
}
```

The compose file already trusts the forwarded client address from a proxy on the same host. [docs/self-hosting.md](./docs/self-hosting.md) covers nginx, trusted proxies, upgrades, backups, and logs.

## Adding Providers

Settings, Providers (or the first-run setup) offers these choices. Every connection lists the provider's models; turn on the ones you want (all of them when there are 20 or fewer).

| Choice | Base URL (default) | Key | Notes |
| --- | --- | --- | --- |
| OpenAI | `https://api.openai.com/v1` | Required | The Responses API: tool calls and reasoning summaries on the newest models. For push to talk and spoken replies, switch the connection's API to Chat Completions and turn on audio in and out on its audio models |
| Anthropic | `https://api.anthropic.com` | Required | Messages API with thinking, prompt caching, and tools; no audio |
| Gemini | `https://generativelanguage.googleapis.com/v1beta/openai` | Required | Google's OpenAI-compatible endpoint; limits and thinking from Google's model list, images and PDFs on by default, audio input when you turn it on |
| OpenRouter | `https://openrouter.ai/api/v1` | Required | Capabilities come from OpenRouter's model list |
| Ollama | `http://localhost:11434/v1` | None | Any self-hosted OpenAI-compatible server works the same way |
| OpenAI-compatible | Your server's `/v1` URL | Optional | vLLM, LM Studio, llama.cpp server, Groq, Together, gateways; Chat Completions, or the Responses API when the server has it |
| Anthropic-compatible | Your gateway's URL | Optional | Gateways that speak the Messages API |

**Ollama and other servers on your network.** In Docker, `localhost` is the container itself, so point the connection at an address the server can reach: `http://ollama:11434/v1` for an Ollama container in the same compose project, or the machine's LAN address. Private addresses are reachable only by connections that `ALLOW_PRIVATE_NETWORK` permits: by default the administrator's (`admins`); set `all` when you trust every account, or `none` to allow public `https` endpoints only. See [Private Network and Ollama](./docs/self-hosting.md#private-network-and-ollama).

Each model has capability switches (vision, PDF, tools, audio in, audio out, reasoning) filled in from the provider's metadata or the preset's defaults; change them on the model when a provider reports nothing.

## Try It without a Key

`tools/fake-llm` is a scriptable fake model server for development: both provider formats plus a small MCP server, on `http://127.0.0.1:4010` with the key `fake-key`. Run it next to a development setup ([Development](#development)):

```bash
make dev        # Postgres, API on :3000, web on :4000
make fake-llm   # in another terminal
```

In the app (as the administrator, since the fake server is on loopback), add an **OpenAI-compatible** connection with the base URL `http://127.0.0.1:4010/v1` or an **Anthropic-compatible** one with `http://127.0.0.1:4010`, key `fake-key`, and a tool server at `http://127.0.0.1:4010/mcp` (tools `echo` and `add`). Words in your message choose the answer:

| Message | Answer |
| --- | --- |
| `hello` (anything else) | `You said: hello`, streamed in chunks with usage |
| `failed 403`, `failed 429`, `failed 500` | That HTTP status with the provider's error body (`429` with `Retry-After`) |
| `midstream error` | Some text, then a broken stream |
| `slow`, `long`, `think` | Slow chunks, a long Markdown reply, reasoning first |
| `tool echo {"text": "hi"}` | A call to the tool server's `echo` (approve it to get `Tool said: hi`) |
| `refuse` | A refusal |
| `audio` (model `fake-audio`, Speak replies on) | A short tone as streamed audio with a transcript |

Every keyword, model, and setting is in [tools/fake-llm/README.md](./tools/fake-llm/README.md). The fake server is a dev tool only: no Copilot crate depends on it, and it never reaches the image.

## Native Apps

The Linux and Android apps wrap the same web app with Tauri 2. On first launch they ask for your server's URL (they warn before using plain `http` to a public address) and then behave like the web app, with downloads through the system's save dialog and the microphone where the model supports audio.

Download them from the [releases page](https://github.com/host39/copilot/releases): each release has `Copilot_<version>_amd64.deb`, `Copilot_<version>_amd64.AppImage`, and `Copilot_<version>_arm64.apk` (release-signed) with a `SHA256SUMS` file (`sha256sum -c SHA256SUMS --ignore-missing`). The **Native apps** workflow (`.github/workflows/native.yml`) builds them on the version tag and attaches them; it also builds on manual dispatch and on pushes that change the apps or the web app, as run artifacts (debug-signed, kept 14 days). The `.deb` needs Ubuntu 24.04 or another system with glibc 2.39 or newer and WebKitGTK 4.1; the APK is for 64-bit ARM phones. Building them yourself, signing, and the device checklist are in [apps/native/README.md](./apps/native/README.md).

## Development

Prerequisites: Rust stable (`rust-toolchain.toml`; rustup installs it on the first build), Node.js 22 (`.nvmrc`) and pnpm 10, Docker for the local Postgres, and Make. Optional: [`watchexec`](https://github.com/watchexec/watchexec) to restart the API on change, and `jq` plus `psql` for `make smoke`.

```bash
make setup      # install, create the .env.local files (with a SECRET_KEY), start Postgres, migrate, seed
make dev        # Postgres, API on :3000 (logs the setup token), web on :4000
```

Open <http://localhost:4000>. The OpenAPI document is at <http://localhost:3000/api/doc> and Swagger UI at <http://localhost:3000/api/ui>.

| Command | Description |
| --- | --- |
| `make setup` | Install, create the env files, start Postgres, migrate, seed |
| `make dev` / `make dev-api` / `make dev-web` | Postgres plus API and web, the API only, or the web dev server only |
| `make fake-llm` | The fake model and MCP server on `:4010` |
| `make migrate` | Apply migrations (lib-rustauth's, then Copilot's) and the role grants |
| `make migrations name=<name>` | Scaffold a migration pair |
| `make seed` | Apply the local SQL seed |
| `make contracts` | Regenerate the TypeScript bindings and conformance fixtures from the Rust contracts |
| `make check` | Format, lint (rustfmt, Clippy, import paths, Biome, markdownlint), type-check, test, boundaries |
| `make ci` | The same without writes, plus contract drift |
| `make test` / `make test-db` | Unit tests; tests that need Postgres (one throwaway database each) |
| `make test-bruno` | Bruno API tests against this checkout's API |
| `make smoke` | The API against the fake model server over HTTP, on a throwaway database |
| `make test-upgrade` | A database written by the previous release tag, upgraded and used by this checkout |
| `make e2e` | The phone-size browser test (Playwright in Google Chrome) against the server and the fake model server |
| `make build` | Release binaries and the production web bundle |
| `make native-dev` / `make android-dev` | The Linux or Android app on the web dev server |
| `make update` / `make clean` | Update dependencies; remove build output, dependencies, and the database volume |

Run `make check` before pushing; CI runs it plus the database tests, Bruno, the smoke test, the upgrade check, the phone-size browser test, the footprint budgets, and (when their inputs change) the image and the native apps. Every environment variable is in [docs/env.md](./docs/env.md); contributor and assistant rules are in [AGENTS.md](./AGENTS.md).

## Architecture

```text
 browser / Linux app / Android app
          |  HTTPS, JSON and server-sent events
          v
 server (axum) ----- providers: OpenAI-compatible, Anthropic   (forward only)
   |  web bundle  `-- MCP tool servers (Streamable HTTP)
   v
 Postgres: accounts (auth schema), chats, projects, files, schedules
```

One process per install holds the runs in flight, the approvals, the scheduler, and the live event streams; everything durable is in Postgres, including file bytes. The Rust code follows a clean, domain-driven layering ([ADR-001](./docs/adr/001-domain-driven-design.md)): the domain (`copilot-core`) defines entities, services, and ports and depends on no framework; adapters implement the ports; the API crate wires them together. `make boundaries` enforces the dependency rules.

| Path | Package | Responsibility |
| --- | --- | --- |
| `apps/api` | `copilot-api` | axum HTTP adapter: composition root, routes under `/api`, run registry, scheduler loop, the web bundle, the `server` binary |
| `apps/web` | `@copilot/web` | Rsbuild + React Router SPA: chats, projects, schedules, settings |
| `apps/e2e` | `@copilot/e2e` | Playwright browser tests at phone size (`make e2e`) |
| `apps/native` | `@copilot/native` + `copilot-native` | Tauri 2 shell for Linux and Android around the web build (outside the Cargo workspace) |
| `packages/core` | `copilot-core` | Domain entities, services, ports, the conversation engine, `DomainError` |
| `packages/database` | `copilot-database` | Postgres pool, migrations, repositories, the `migrate` and `seed` binaries |
| `packages/providers` | `copilot-providers` | OpenAI-compatible and Anthropic wire protocols (implements `LlmGateway`) |
| `packages/mcp` | `copilot-mcp` | MCP client over Streamable HTTP with rmcp (implements `ToolHost`) |
| `packages/security` | `copilot-security` | Secret sealing and the hardened outbound HTTP clients (address policy, no redirects, caps) |
| `packages/contracts` | `copilot-contracts` + `@copilot/contracts` | Shared DTOs: OpenAPI schemas, generated TypeScript types, Zod schemas |
| `packages/middleware` | `copilot-middleware` | CORS, error mapping, request ids, request logging |
| `packages/logger` | `copilot-logger` | tracing subscriber (readable locally, JSON elsewhere) |
| `tools/fake-llm` | `fake-llm` | Dev only: the scriptable fake model and MCP server |

The design is [docs/design/copilot.md](./docs/design/copilot.md); the decisions behind it are in [ADR-005](./docs/adr/005-copilot-architecture.md).

### Identity (lib-rustauth)

Accounts, sessions, passwords, roles, and user administration come from [lib-rustauth](https://github.com/host39/lib-rustauth) (MIT), a reusable authentication library for axum and Postgres, pinned by commit as a Cargo git dependency. It owns the `auth` schema and its own migrations, which `migrate` applies before Copilot's; Copilot's tables reference `auth.users`. Only `apps/api/src/helpers/auth.rs` names it, and the domain crate never depends on it. Changes Copilot needs go into lib-rustauth as generic features, never as Copilot-specific code.

## Documentation

| Topic | Where |
| --- | --- |
| Self-hosting: compose, secrets, proxy, stdio MCP servers, upgrades, backups, logs | [docs/self-hosting.md](./docs/self-hosting.md) |
| Release notes and upgrade notes | [CHANGELOG.md](./CHANGELOG.md) |
| Environment variables | [docs/env.md](./docs/env.md) |
| Footprint: sizes, crates, memory | [docs/footprint.md](./docs/footprint.md) |
| Product and architecture design | [docs/design/copilot.md](./docs/design/copilot.md), build plan in [docs/design/roadmap.md](./docs/design/roadmap.md) |
| Decisions | [docs/adr/](./docs/adr/README.md) |
| Research (providers, voice, MCP, native apps, dependencies) | [docs/research/](./docs/research/) |
| OpenAPI and Swagger UI | [docs/openapi.md](./docs/openapi.md) |
| Native apps | [apps/native/README.md](./apps/native/README.md) |
| Contributing with AI assistants | [AGENTS.md](./AGENTS.md) (rules in `.ai/rules/`, skills in `.claude/skills/`) |

## License

Copilot is licensed under the [PolyForm Noncommercial License 1.0.0](./LICENSE): use, change, and share it for any noncommercial purpose, as long as the license terms and its `Required Notice` line travel with the code. Commercial use needs a separate license from the copyright holder. lib-rustauth, a separate project, is MIT licensed.
