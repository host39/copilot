# AGENTS.md

Monorepo starter: Rust (axum) API + Rsbuild/React web on clean/DDD layering. Health-check endpoints only; the structure is the product.

## Commands

Run these yourself from the repo root to verify work:

| Command | Checks |
| --- | --- |
| `make lint` | rustfmt, Clippy (`-D warnings`), Rust import paths, Biome, markdownlint |
| `make type-check` | `cargo check` (all targets, all features) + `tsc` |
| `make test` | `cargo test --workspace` + TypeScript tests |
| `make boundaries` | Crate dependency rules in `boundaries.config.mjs` |
| `make contracts` | Regenerates `packages/contracts/ts/generated/` and the conformance fixtures in `packages/contracts/test/fixtures/` after contract changes |
| `make check` | format + lint + type-check + test + boundaries |
| `make test-db` | Tests marked `#[ignore]` that need Postgres (`#[sqlx::test]`, one throwaway database per test) |
| `make test-bruno` | Bruno API tests (needs a migrated database; starts this checkout's API on its `PORT` and refuses to reuse a running server unless `BASE_URL` is set). Registers a run-unique account, which becomes the administrator of a database without accounts |
| `make migrations name=<name>` | Scaffolds a migration pair |
| `make migrate` | Applies migrations + readonly role grants, and the server role's when `DB_SERVER_PASSWORD` is set (needs Postgres) |

Run `make check` before considering work done. If a check needs infrastructure that is not running (Postgres for `test-db` / `test-bruno`), say so instead of skipping it silently.

**User-only** (these start Docker, change the database, or delete data): give the user the command instead of running it.

| Command | Purpose |
| --- | --- |
| `make setup` | Install, copy env, start Postgres, migrate, seed |
| `make dev` / `make dev-api` | Start Postgres and run the apps |
| `make seed` | Apply local SQL seed (`packages/database/seed/local/`) |
| `make clean` | Remove build output, dependencies, and the database volume |

After bumping `@biomejs/biome` in the catalog, run `pnpm biome migrate --write` if Biome reports deprecated configuration fields. Keep the local `$schema` path (`node_modules/@biomejs/biome/configuration_schema.json`) in `biome.json` files.

## Package Map

| Path | Role |
| --- | --- |
| `apps/api` | axum HTTP adapter: composition root (`helpers/bootstrap.rs`), the lib-rustauth adapter (`helpers/auth.rs`, the only module that names `rustauth`), the run registry (`helpers/runs.rs`), the scheduler loop (`helpers/scheduler.rs`), OpenAPI routes under `/api` (core to wire mapping in `routes/convert.rs`), the web bundle (`WEB_DIR`), the `server` binary |
| `apps/web` | Rsbuild + React Router SPA |
| `packages/core` | Domain entities, services, repository traits (ports), `DomainError` |
| `packages/database` | Postgres pool, env parsing, sqlx migrations, seed, repository implementations, `migrate` and `seed` binaries |
| `packages/contracts` | Shared DTOs: Rust structs (source of truth, OpenAPI schemas) + generated TS types + Zod schemas; re-exports `@rustauth/contracts` for the web app; Rstest tests in `test/` |
| `packages/middleware` | CORS, error handler, request id, request logger, `JsonBody` extractor (authentication comes from lib-rustauth's guard) |
| `packages/logger` | tracing subscriber setup |
| `packages/providers` | `copilot-providers`: OpenAI-compatible and Anthropic wire protocols over reqwest (implements `LlmGateway`) |
| `packages/mcp` | `copilot-mcp`: MCP client over Streamable HTTP with rmcp (implements `ToolHost`) |
| `packages/security` | `copilot-security`: AES-256-GCM sealing (`SecretCipher`) and the hardened outbound reqwest client builder |

The three adapter crates use only core ports and the outbound client the composition root injects. The Tauri shell (`apps/native/src-tauri`, Phase 5) is excluded from the Cargo workspace.

Identity comes from lib-rustauth (`host39/lib-rustauth`, a Cargo git dependency): users, sessions, passwords, roles, user administration, and the `auth` schema. `copilot-core` never depends on it.

## Non-Negotiables

- Domain logic lives in `copilot-core`; sqlx adapters in `copilot-database`. No domain logic in repositories or handlers.
- `copilot-core` must not depend on `axum`, `sqlx`, `utoipa`, `tokio`, `reqwest`, `rmcp`, contracts, database, middleware, logger, the adapter crates (providers, mcp, security), rustauth crates, or apps (`make boundaries`). Only `apps/api` and `copilot-database` may depend on lib-rustauth, through its `rustauth` facade; `copilot-database` only with the `migrate` feature, for the `auth` migrations.
- Services fail with `DomainError` variants carrying human-readable messages; HTTP mapping lives in `copilot-middleware` (`HttpError`). Infrastructure failures use `DomainError::internal("<dotted.key>", error)`.
- Wire services in the composition root (`ApiContext::create` / `create_app`), store them in `AppState`, and never construct repositories in handlers.
- Contracts are for the API and the web app; core does not depend on contracts. After changing a contract struct, update its samples in `packages/contracts/tests/fixtures.rs`, run `make contracts`, and update the Zod schema in `packages/contracts/ts/index.ts` (it must `satisfies` the generated type and parse every sample in `test/fixtures/`).
- `apps/web` only imports `@copilot/contracts` and talks to the API over HTTP.
- Log with `tracing` macros and dotted keys; no `println!` / `eprintln!` / `dbg!` (Clippy denies them) and no `console.*` outside `apps/web/src/logging.ts` (Biome denies it).
- TypeScript uses alias imports only (no `./` or `../`); Rust uses `crate::` paths (no `super::` outside test modules).
- Write schema changes as new migrations (`make migrations name=<name>`) and apply them with `make migrate`. Do not edit a migration after it has been applied to a shared or deployed database: sqlx checksums applied migrations and refuses to run when one changes.
- Dev uses `apps/api/.env.local` (API port **3000**); the web dev server runs on **4000**.

## Where to Look

- Architecture and domain workflow: [docs/adr/001-domain-driven-design.md](./docs/adr/001-domain-driven-design.md)
- Rust stack decisions: [docs/adr/003-rust-backend-stack.md](./docs/adr/003-rust-backend-stack.md)
- Contracts and TypeScript bindings: [docs/adr/004-contracts-and-type-sharing.md](./docs/adr/004-contracts-and-type-sharing.md)
- Migrations and seed: [docs/adr/002-database-migrations-and-seed.md](./docs/adr/002-database-migrations-and-seed.md)
- Environment variables: [docs/env.md](./docs/env.md)
- Dependency boundaries: [boundaries.config.mjs](./boundaries.config.mjs)

## AI Assistants

| Kind | Location |
| --- | --- |
| Rules | `.ai/rules/*.md`, loaded by path through `.claude/rules/*.md` and `.cursor/rules/*.mdc` wrappers |
| Skills | `.claude/skills/`: `/add-domain`, `/architecture`, `/api-response-patterns`, `/bruno-tests`, `/dotted-error-messages`, `/undraw-illustrations`, `/agent-workflows` |
| Permissions | `.claude/settings.json`: checks and `make migrate` pre-allowed; other infrastructure and database commands ask first |

Use skills when adding domains, API routes, Bruno tests, or reviewing layer boundaries. Use `/agent-workflows` whenever you plan or run multiple agents, reviews, or CI jobs: it holds the owner's standing limits (2 agents at a time, risk-based reviews, when a smaller model is allowed, smallest CI runners). Dotted keys are for logs and infrastructure errors only; client-facing `DomainError` messages stay human-readable.

## Project Rules

| Rule | Topic |
| --- | --- |
| `import-paths` | TypeScript alias imports; no `./` or `../` |
| `rust-imports` | `crate::` paths; no `super::` outside test modules |
| `naming` | Short names; no `From` / `Of` connectors in function names |
| `postgres-column-alignment` | Column order in `CREATE TABLE` for row alignment |
| `structured-logging` | Dotted keys for tracing logs and infrastructure errors |
| `markdown-title-case` | Title case for Markdown headings |
| `document-titles` | `Copilot \| <Action>` via `PageTitle` |
| `shared-css-styles` | Shared Tailwind classes in `main.css`, not TS exports |
| `parallel-db-reads` | Parallel SELECTs outside transactions, bounded by the pool |
