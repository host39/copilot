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
| `packages/core` (`copilot-core`) | Entities, services, repository traits (ports), `DomainError`, shared types | `axum`, `sqlx`, `utoipa`, `tower*`, `hyper`, `tokio`, `reqwest`, `rmcp`, `sse-stream`, `aes-gcm`, `argon2`, database, contracts, middleware, logger, adapters, rustauth crates, apps |
| `packages/database` (`copilot-database`) | Postgres pool, env parsing, migrations, seed, sqlx repositories | apps, middleware, adapters, rustauth's inner crates (the `rustauth` facade only with the `migrate` feature), `axum`, `utoipa` |
| `packages/providers` (`copilot-providers`) | OpenAI-compatible and Anthropic wire protocols (`LlmGateway`) | apps, database, contracts, middleware, other adapters, rustauth crates, `axum`, `sqlx`, `utoipa` |
| `packages/mcp` (`copilot-mcp`) | MCP client over Streamable HTTP with rmcp (`ToolHost`) | same as providers |
| `packages/security` (`copilot-security`) | Secret sealing (`SecretCipher`), hardened outbound reqwest client builder | same as providers |
| `packages/contracts` (`copilot-contracts` + `@copilot/contracts`) | Request/response DTOs, OpenAPI schemas, TS bindings, Zod schemas | core, database, adapters, rustauth crates, apps, `axum`, `sqlx` |
| `packages/middleware` (`copilot-middleware`) | CORS, error handler, request id, request logger, `JsonBody` | database, adapters, rustauth crates, `sqlx` |
| `packages/logger` (`copilot-logger`) | tracing subscriber setup | database, `sqlx` |
| `apps/api` (`copilot-api`) | Composition root, OpenAPI routes, entrypoints | (may use every crate above) |
| `apps/web` (`@copilot/web`) | API clients, pages, components | anything but `@copilot/contracts`; reaches the API over HTTP only |

Adapters ("providers", "mcp", "security") implement core ports. The provider and MCP adapters receive an `Outbound<reqwest::Client>` that the composition root builds once per adapter with `copilot_security::clients`, pick a client per request with core's `Outbound::pick`, and cap responses with core's `read_capped` and `cap_events`; they never build their own client or depend on `copilot-security`. Identity comes from lib-rustauth: only `apps/api` and `copilot-database` (with the `migrate` feature, for the `auth` migrations) depend on it, both through the `rustauth` facade.

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

`apps/api/src/helpers/bootstrap.rs` (`ApiContext::create`, async) parses config, installs the logger, creates the pool, starts lib-rustauth (`helpers::auth::Identity`, the only module that names `rustauth`), and calls `create_app`, which builds every service once in `wire` on `PgStore`, `AesCipher`, and the outbound adapters, stores them in `AppState`, and mounts the routes under `/api` behind the guard. Handlers read services via `State<AppState>` and the caller via `Caller`. Route handlers never construct repositories or pools.

## Verification

```bash
make boundaries
```
