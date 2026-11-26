# Dotted Operational Key Examples

Use these for **logs and infrastructure errors only**, not for client-facing `DomainError` messages.

## API

```rust
tracing::info!(host, port, environment = %config.environment, "api.server.listening");
tracing::info!(signal = "SIGTERM", "api.server.shutdown");
// Before the logger exists, binaries print the key and detail to stderr:
// api.server.start.failed api.config.env.invalid ENVIRONMENT
```

## Database

```rust
DomainError::internal("database.widgets.list.failed", error)

#[error("database.readonly.unknown-omitted-column")]
UnknownOmittedColumn,

tracing::info!(migrations, "database.migrate.applied");
tracing::warn!(folder, "database.seed.skipped.no-files");
```

## Middleware

```rust
tracing::warn!(status, code, reason, request_id, "middleware.error.client");
tracing::warn!(path, reason = %error, request_id, "middleware.auth.rejected");
tracing::info!(method, path, status, duration_ms, request_id, "middleware.http.ok");
```

## Web

```typescript
log("warn", "web.api.request.failed", { path, status, code, requestId });
log("error", "web.error-boundary.caught", { reason: error.message });
```

## Domain (Human, Not Dotted)

```rust
// GOOD
DomainError::validation("Name is required.").with_field("name")

// BAD for a client-facing error
DomainError::validation("core.widget.name.invalid.required")
```
