# Domains

Each business domain gets its own module here, following the clean / DDD layout used across our projects:

```text
domains/<domain>/
├── entities/                 # Rich domain types with invariants (no I/O)
│   └── mod.rs
├── repositories/             # Repository traits (ports), no database code
│   └── mod.rs
├── services/                 # Application services orchestrating the domain
│   └── mod.rs
└── mod.rs                    # Public exports for the domain (the barrel)
```

## Rules

- Entities and services contain business logic only. They never use `axum`, `sqlx`, or any framework crate.
- Repository traits live here; their `sqlx` implementations live in `copilot-database`.
- Services return `DomainResult<T>` and fail with `DomainError` variants that carry human-readable messages. HTTP mapping lives in `copilot-middleware`.
- Re-export a domain's public API from `packages/core/src/lib.rs`.
- Colocate unit tests in a `#[cfg(test)] mod tests` block at the bottom of the file under test.

See `docs/adr/001-domain-driven-design.md` and the `/add-domain` skill for the full workflow of adding a domain end to end.
