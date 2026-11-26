# Structured Logging Keys

Operational logs use stable dot-notation keys so operators can filter on the message and correlate requests with `request_id` (the `X-Request-Id` header).

## Keys

Use `service.area.outcome` as the message:

- Lowercase segments separated by **dots**
- Multi-word segments use **kebab-case** (`middleware.error.internal`, `middleware.http.client-error`)
- No underscores in keys
- IDs, paths, status codes, and errors go in **fields**, never in the key
- **Bounded detail** may follow the key after a space in error messages (`database.env.invalid DB_HOST, DB_PORT`)

## Rust

Log through the `tracing` macros. The message string is the key; context is structured fields with `snake_case` names. Use `%value` for `Display` and `?value` for `Debug`.

```rust
// GOOD
tracing::info!(host, port, "api.server.listening");
tracing::error!(error = %error, request_id, "middleware.error.internal");

// BAD
tracing::info!("Server listening on {host}:{port}");
tracing::warn!("HTTP client error");
```

`println!`, `eprintln!`, and `dbg!` are denied workspace-wide by Clippy. The only exception is an `#[allow(clippy::print_stderr)]` block in a binary that reports a failure before the logger is installed.

## Infrastructure Errors

Persistence, config, and integration failures carry the same dotted keys and surface as `500` responses with a generic body:

```rust
DomainError::internal("database.widgets.insert.failed", error)

#[derive(Debug, thiserror::Error)]
pub enum GrantError {
    #[error("database.readonly.invalid-identifier")]
    InvalidIdentifier,
}
```

## API and Domain Errors

Do **not** use dotted keys as client-facing `DomainError` messages. Use human-readable sentences; put machine identifiers in `code`, `field`, and `context`. See the `/dotted-error-messages` skill.

## Web

`apps/web/src/logging.ts` is the only module allowed to use `console` (Biome `noConsole` enforces this): `log("warn", "web.api.request.failed", { status, requestId })`.

## Out of Scope

- Test-only panics and assertion messages
- Programming invariant panics (`expect("...")` explaining why a value must exist)
