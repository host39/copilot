---
name: architecture
description: DDD layer boundaries and dependency direction for this workspace. Use when adding crates or packages, wiring dependencies across layers, reviewing architecture, or unsure where code belongs (core vs database vs contracts vs middleware vs apps).
paths:
  - "packages/**"
  - "apps/**"
---

# Architecture

## When to Use

- Adding or moving code between crates
- Reviewing dependency direction or boundaries
- Unsure whether logic belongs in core, database, contracts, middleware, or an app

Dependencies point inward: adapters depend on the domain, never the reverse. See [docs/adr/001-domain-driven-design.md](../../../docs/adr/001-domain-driven-design.md).

## Layers

| Crate / package | Contains | Must NOT depend on |
| --- | --- | --- |
| `packages/core` (`copilot-core`) | Entities, services, repository traits (ports), `DomainError`, shared types | `axum`, `sqlx`, `utoipa`, `tower*`, `hyper`, database, contracts, middleware, logger, apps |
| `packages/database` (`copilot-database`) | Postgres pool, env parsing, migrations, seed, sqlx repositories | apps, middleware, `axum`, `utoipa` |
| `packages/contracts` (`copilot-contracts` + `@copilot/contracts`) | Request/response DTOs, OpenAPI schemas, TS bindings, Zod schemas | database, apps, `axum`, `sqlx` |
| `packages/middleware` (`copilot-middleware`) | CORS, error handler, request id, request logger, `JsonBody` | database, `sqlx`, rustauth crates |
| `packages/logger` (`copilot-logger`) | tracing subscriber setup | database, `sqlx` |
| `apps/api` (`copilot-api`) | Composition root, OpenAPI routes, entrypoints | (may use every crate above) |
| `apps/web` (`@copilot/web`) | API clients, pages, components | anything but `@copilot/contracts`; reaches the API over HTTP only |

`boundaries.config.mjs` encodes these rules; `make boundaries` checks them against `cargo metadata`. Read each rule's `comment` when unsure.

## Logging Port

Every crate logs through the `tracing` macros (the facade); only `copilot-logger` installs a subscriber. Core may use `tracing` but never `tracing-subscriber` or the logger crate.

## Error Handling

```rust
// BAD: build an HTTP response in a handler for a domain failure
return (StatusCode::NOT_FOUND, Json(json!({ "error": "Not found" }))).into_response();

// GOOD: fail in the service; HttpError maps DomainError to status + body
return Err(DomainError::not_found("Widget", id.to_string()));
```

Infrastructure failures use `DomainError::internal("<dotted.key>", error)` and become `500` with a generic message. HTTP envelopes: `/api-response-patterns`. Log keys: `structured-logging` rule.

## Composition Root

`apps/api/src/helpers/bootstrap.rs` (`ApiContext::create`) parses config, installs the logger, creates the pool, and calls `create_app`. Services and repositories are built there or in `create_app`, stored in `AppState`, and read by handlers via `State<AppState>`. Route handlers never construct repositories or pools.

## Verification

```bash
make boundaries
```
