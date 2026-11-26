---
name: dotted-error-messages
description: >-
  Enforces dotted-notation keys for tracing logs and infrastructure errors; forbids dotted keys as
  client-facing DomainError messages. Use when adding logs, config or database errors, or when
  reviewing error message style.
paths:
  - "apps/**"
  - "packages/**"
---

# Dotted Operational Keys (Not DomainError Messages)

Errors and logs use **two channels** (see the `structured-logging` rule):

| Channel | Message style | Examples |
| --- | --- | --- |
| `tracing` macros, `DomainError::Internal`, infrastructure error enums | Dotted keys | `middleware.error.internal`, `database.env.invalid DB_HOST` |
| Every other `DomainError` variant | Human-readable | `"Name is required."`, `"Widget with id '42' not found"` |

Do **not** use dotted keys as client-facing `DomainError` messages, and do not put human prose in log keys.

## Log Keys

```text
<app-or-crate>.<area>.<outcome>
```

- Lowercase segments, kebab-case within a segment
- IDs, paths, and errors go in **fields**, not in the key

```rust
tracing::error!(status, code, reason, request_id, "middleware.error.internal");
tracing::info!(host, port, "api.server.listening");
```

## Infrastructure Errors

```rust
// Repository adapters
DomainError::internal("database.widgets.insert.failed", error)

// Error enums
#[derive(Debug, thiserror::Error)]
pub enum SeedError {
    #[error("database.seed.read.failed {path}")]
    Read { path: PathBuf, #[source] source: std::io::Error },
}

// Config parsing collects every invalid variable after the key
// "api.config.env.invalid PORT, LOG_LEVEL"
```

`HttpError` logs the key and source for `Internal` and answers `500` with a generic message, so keys never reach clients.

## Domain Errors (Human Messages)

```rust
DomainError::validation("Name is required.").with_field("name")
DomainError::not_found("Widget", id.to_string())
DomainError::business_rule("Archived widgets cannot be renamed.", "WIDGET_ARCHIVED")
DomainError::forbidden("Only the owner can delete this widget.")
```

Use `code`, `field`, and `with_context` for machines, not dotted prose in the message.

## Crate Prefix Hints

| Source path | Prefix |
| --- | --- |
| `apps/api/` | `api` |
| `apps/web/` | `web` |
| `packages/database/` | `database` |
| `packages/middleware/` | `middleware` |
| `packages/logger/` | `logger` |
| `scripts/` | `scripts` |

## Additional Resources

- Examples: [examples.md](examples.md)
- Full rule: [`.ai/rules/structured-logging.md`](../../../.ai/rules/structured-logging.md)
