<!-- markdownlint-disable MD033 MD041 -->

<div align="center">

# Copilot

<img src="docs/images/readme-hero.svg" alt="Full-stack development illustration" width="420" />

[![Rust](https://img.shields.io/badge/Rust-2024-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![axum](https://img.shields.io/badge/axum-0.8-6E4A7E)](https://github.com/tokio-rs/axum)
[![sqlx](https://img.shields.io/badge/sqlx-Postgres-336791?logo=postgresql&logoColor=white)](https://github.com/launchbadge/sqlx)
[![React](https://img.shields.io/badge/React-19-61DAFB?logo=react&logoColor=black)](https://react.dev/)
[![pnpm](https://img.shields.io/badge/pnpm-10-f69220?logo=pnpm&logoColor=white)](https://pnpm.io/)
[![Biome](https://img.shields.io/badge/Biome-2.5-60A5FA?logo=biome&logoColor=white)](https://biomejs.dev/)
[![Docker](https://img.shields.io/badge/Docker-Postgres-2496ED?logo=docker&logoColor=white)](https://www.docker.com/)
[![License](https://img.shields.io/badge/license-PolyForm%20Noncommercial%201.0.0-lightgrey)](./LICENSE)

</div>

A monorepo starter for full-stack projects: a Rust (axum) API and a React web app on top of shared packages that follow a clean / domain-driven architecture. It ships with health-check endpoints only: a clean starting point rather than a specific product.

---

## Prerequisites

- Rust stable (see `rust-toolchain.toml`; rustup installs it on first build)
- Node.js 22 (see `.nvmrc`) and pnpm 10
- Docker (for the local Postgres database)
- Make (optional, recommended)
- Optional: [`watchexec`](https://github.com/watchexec/watchexec) (`cargo install --locked watchexec-cli`) to restart the API on change

---

## Quick Start

```bash
make setup          # install, copy .env.local files, start Postgres, migrate + seed
make dev            # Postgres, API on :3000 + web on :4000
```

Open <http://localhost:4000>; the home page calls the API's `/health` endpoint and shows its status. The OpenAPI document is at <http://localhost:3000/doc> and Swagger UI at <http://localhost:3000/ui>.

For a single app:

```bash
make dev-api        # starts Postgres, API only on :3000
make dev-web        # web only on :4000
```

---

## Make Commands

| Command | Description |
| --- | --- |
| `make install` | Install pnpm and Cargo dependencies |
| `make update` | Update dependencies within semver ranges |
| `make update-latest` | Update pnpm dependencies to latest stable; bump Cargo versions in `Cargo.toml` by hand |
| `make setup` | Install, copy env, start DB, migrate, seed |
| `make dev` | Start Postgres, then run API + web in parallel |
| `make dev-api` | Start Postgres, then run the API only |
| `make dev-web` | Run the web dev server only |
| `make migrate` | Apply migrations + readonly grants |
| `make seed` | Apply local SQL seed files |
| `make migrations` | Scaffold a migration pair (`name=create_widgets`) |
| `make contracts` | Regenerate TypeScript bindings from the Rust contracts |
| `pnpm db:stop` / `pnpm db:reset` | Stop or reset the Postgres container |
| `make test` | Rust and TypeScript unit tests |
| `make test-db` | Tests that need Postgres (one throwaway database per test) |
| `make test-bruno` | Bruno API tests against this checkout's API (`BASE_URL=<url>` tests a server that is already running) |
| `make lint` | rustfmt, Clippy, Rust import paths, Biome, markdownlint |
| `make format` | Auto-fix lint and format |
| `make type-check` | `cargo check` + TypeScript across the workspace |
| `make boundaries` | Enforce crate dependency rules |
| `make check` | format + lint + type-check + test + boundaries |
| `make ci` | Same checks without writes, plus contract drift |
| `make build` | Release binaries + production web bundle |
| `make clean` | Wipe build output, dependencies, and the database volume |

After `make update` or `make update-latest`, run `make check` to catch breaking changes.

---

## Bootstrapping a New Project

Use the bootstrap script to copy the template into a new repository with your own names:

```bash
node scripts/create-project.mjs ../my-app \
  --name my-app \
  --title "My App"
```

This copies everything (excluding `target`, `node_modules`, `.git`, and build output) and rewrites:

- `@copilot/` npm scope -> `@my-app/` (or `--scope`)
- `copilot-` crate names -> `my-app-` and `copilot_` crate paths -> `my_app_`
- `copilot` database and package name -> `my_app`
- `Copilot` display name -> the `--title` value

Pass `--skip-git-init` to keep your own git setup.

---

## Package Map

| Path | Package | Responsibility |
| --- | --- | --- |
| `apps/api` | `copilot-api` | axum HTTP adapter; composition root, OpenAPI routes, the `server` entrypoint |
| `apps/web` | `@copilot/web` | Rsbuild + React Router SPA (minimal placeholder) |
| `packages/core` | `copilot-core` | Domain entities, services, ports, shared errors and types |
| `packages/database` | `copilot-database` | Postgres pool, env, migrations, seed, sqlx repositories |
| `packages/middleware` | `copilot-middleware` | CORS, error handler, request id, request logger |
| `packages/logger` | `copilot-logger` | tracing subscriber (readable locally, JSON elsewhere) |
| `packages/providers` | `copilot-providers` | LLM provider adapters (OpenAI-compatible, Anthropic) |
| `packages/mcp` | `copilot-mcp` | MCP client over Streamable HTTP (rmcp) |
| `packages/security` | `copilot-security` | Secret sealing and the hardened outbound HTTP client |
| `packages/contracts` | `copilot-contracts` + `@copilot/contracts` | Shared DTOs: OpenAPI schemas, TypeScript bindings, Zod schemas |

---

## Environment Variables

See [docs/env.md](./docs/env.md) for the full reference. `make setup` copies `apps/api/.env.sample` to `apps/api/.env.local` and `apps/web/.env.example` to `apps/web/.env.local`.

---

## Documentation

- **Architecture**: [docs/adr/001-domain-driven-design.md](./docs/adr/001-domain-driven-design.md) (layering, dependency rules, adding a domain end to end)
- **Rust backend stack**: [docs/adr/003-rust-backend-stack.md](./docs/adr/003-rust-backend-stack.md)
- **Contracts shared with the web app**: [docs/adr/004-contracts-and-type-sharing.md](./docs/adr/004-contracts-and-type-sharing.md)
- **Migrations and seed**: [docs/adr/002-database-migrations-and-seed.md](./docs/adr/002-database-migrations-and-seed.md)
- **Environment variables**: [docs/env.md](./docs/env.md)
- **OpenAPI / Swagger**: [docs/openapi.md](./docs/openapi.md)
- **Deployment**: [container](./docs/deployment-container.md) (one API process per install; see [ADR-005](./docs/adr/005-copilot-architecture.md))
- **AI assistants**: [AGENTS.md](./AGENTS.md) (Cursor; Claude Code via [CLAUDE.md](./CLAUDE.md))
  - **Rules** (auto-applied by path): `.ai/rules/*.md` via `.cursor/rules/` or `.claude/rules/` wrappers
  - **Skills** (on demand): `/add-domain`, `/architecture`, `/api-response-patterns`, `/bruno-tests`, `/dotted-error-messages`, `/undraw-illustrations` in `.claude/skills/`

---

## License

[PolyForm Noncommercial License 1.0.0](./LICENSE). Noncommercial use, changes, and redistribution are allowed as long as the license terms and the `Required Notice` line travel with the code. Commercial use needs a separate license from the copyright holder.

Projects created with `scripts/create-project.mjs` copy this `LICENSE`; replace it if a new project needs different terms.
