# Domains

Each business domain gets its own module here, following the clean/DDD layout of `docs/design/copilot.md` (Domain Model):

```text
domains/<domain>/
├── entities.rs     # Entities and value objects with their invariants (no I/O)
├── ports.rs        # Repository traits (ports) and their query/report types, no database code
├── service.rs      # Application service orchestrating the use cases
└── mod.rs          # Public exports for the domain (the barrel)
```

A domain may split a large file further (for example `provider/capabilities.rs` or a second service in `provider/model_service.rs`). Submodules stay private; other modules import from the domain barrel or the crate root.

## Shared Types and Ports

- `shared/`: typed ids (`UserId`, `ProjectId`, ...), `Actor` (the caller, built by `apps/api` from lib-rustauth's user), `Secret`, the network policy, `BaseUrl`, `ReasoningLevel`, errors, and constants.
- `ports/`: ports used by several domains (`Clock`, `SecretCipher`, `LlmGateway`, later `ToolHost` and `TurnSink`). There are no identity ports: lib-rustauth owns users, passwords, and sessions.
- `testing/` (feature `testing`): fakes for every port. `MemoryStore` implements every repository trait over one shared in-memory store.

## Rules

- Entities and services contain business logic only. They never use `axum`, `sqlx`, `tokio`, `reqwest`, or any rustauth crate.
- Repository traits live here; their `sqlx` implementations live in `copilot-database`. Every method on owner-scoped data takes the caller's `UserId`, and a row of another user reads as missing (`DomainError::EntityNotFound`, 404).
- Service methods that act for a user take `&Actor`. Services load every referenced id with the caller's `UserId` before saving.
- Services return `DomainResult<T>` and fail with `DomainError` variants that carry human-readable messages. HTTP mapping lives in `copilot-middleware`.
- Re-export a domain's public API from `packages/core/src/lib.rs`.
- Colocate unit tests in a `#[cfg(test)] mod tests` block at the bottom of the file under test, using the fakes from `crate::testing`.

See `docs/adr/001-domain-driven-design.md` and the `/add-domain` skill for the full workflow of adding a domain end to end.
