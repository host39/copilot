# ADR-003: Rust Backend Stack

## Status

Accepted

## Context

The template keeps the layout and tooling of our TypeScript starter (Hono API, React web, shared packages) but replaces the API with Rust. Each TypeScript building block needs a Rust counterpart that keeps the same responsibilities, so the architecture, rules, and skills carry over.

## Decision

| Concern | TypeScript starter | This template |
| --- | --- | --- |
| HTTP framework | Hono | axum on Tokio |
| Middleware | Hono middleware | axum `from_fn` middleware + `tower-http` CORS |
| OpenAPI | `@hono/zod-openapi` + Swagger UI | `utoipa` + `utoipa-axum`; Swagger UI page loads assets from a CDN |
| Database | Drizzle + `pg` | sqlx (Postgres, runtime-checked queries, embedded migrations) |
| Logging | `Logger` port + console / pino | `tracing` facade + `tracing-subscriber` (readable locally, JSON elsewhere) |
| Errors | `DomainError` subclasses | `DomainError` enum (`thiserror`) mapped by `HttpError` |
| Config | Zod env schemas | Small env parser that collects every invalid variable |
| Contracts | Zod schemas shared by API and web | Rust DTOs (source of truth) + ts-rs bindings + Zod schemas that satisfy them ([ADR-004](./004-contracts-and-type-sharing.md)) |
| Tests | Vitest | `cargo test` (colocated `#[cfg(test)]` modules), Bruno for HTTP |
| Lint / format | Biome, markdownlint | rustfmt + Clippy (`-D warnings`) for Rust; Biome and markdownlint for the rest |
| Boundaries | dependency-cruiser | `boundaries.config.mjs` checked against `cargo metadata` |
| Serverless | `hono/aws-lambda` | `lambda_http`, behind the `lambda` Cargo feature |

Further choices:

- **Runtime-checked queries** (`sqlx::query_as`) rather than compile-time macros, so builds and CI never need a live database or an offline query cache.
- **Lazy connection pool**: the API starts while Postgres is down and reports it through `/health/ready`.
- **Request context** (request id) uses a Tokio task-local, the counterpart of `AsyncLocalStorage`, so error bodies and log lines carry the id without threading it through handlers.
- **Workspace lints** forbid `unsafe` and deny `println!`, `eprintln!`, and `dbg!` so all output goes through the logger.

## Consequences

### Positive

- Same layering, commands, and AI rules as the TypeScript starter; projects can move between the two.
- One static binary per entrypoint, small container images, and predictable latency.

### Negative

- Two toolchains (Cargo and pnpm) to install and keep updated.
- Longer cold builds than a TypeScript project; incremental builds and `cargo check` stay fast.

### Neutral

- The web app is unchanged from the TypeScript starter apart from contract imports.

## Alternatives Considered

| Alternative | Why rejected |
| --- | --- |
| actix-web | Mature, but its own actor-based ecosystem; axum composes with `tower` middleware used across Tokio projects |
| Diesel | See [ADR-002](./002-database-migrations-and-seed.md) |
| `utoipa-swagger-ui` | Downloads Swagger UI during the build, which breaks offline and locked-down builds |
| Compile-time checked sqlx macros | Need `DATABASE_URL` or a committed `.sqlx` cache for every build |

## References

- [ADR-001: Domain-Driven Design](./001-domain-driven-design.md)
- [axum](https://docs.rs/axum), [sqlx](https://docs.rs/sqlx), [utoipa](https://docs.rs/utoipa), [tracing](https://docs.rs/tracing)
