# ADR-002: Database Migrations and Local Seed

## Status

Accepted

## Context

Schema changes must be versioned and applied the same way locally, in CI, and on every release. The API must not own migrate or seed logic. Local development needs repeatable fixtures, and a read-only database role should exist for reporting or assistant tooling without access to secrets.

With sqlx there is no schema DSL to generate migrations from: the migration SQL is the schema change.

## Decision

### Migrations

- Migrations are reversible SQL pairs (`<timestamp>_<name>.up.sql` / `.down.sql`) in the database package, scaffolded with `make migrations name=<name>` and embedded into the `migrate` binary at compile time.
- **Committed migrations are append-only.** sqlx checksums every applied migration and refuses to run when a committed file changes; mistakes are fixed with a new migration.
- AI assistants may draft new, uncommitted migrations but never change committed ones or apply migrations. A pre-tool hook enforces this for Claude Code and Cursor.
- `make migrate` (the `migrate` binary) applies migrations, then creates or updates the `readonly` role: it may connect, has `default_transaction_read_only` and a statement timeout, and gets `SELECT` only on tables registered in the schema registry (optionally excluding sensitive columns).
- The API runtime reads and writes data only; it never runs migrations. Deployments run `migrate` as a one-off step before rolling out new code.

### Local Seed (SQL)

- Seed data lives in the database package under `seed/<folder>/` (`local/` for `make seed`, `dev/` for a shared development database), applied in byte-wise filename order.
- The `seed` binary refuses to run when `ENVIRONMENT=production`. Optional skip: `SEED_SKIP_IF_POPULATED=1` plus `SEED_SKIP_QUERY`.
- `make setup` runs migrate, then seed. Seed files are idempotent.

### Code Boundaries

| Concern | Consumed by |
| --- | --- |
| Migrations | `migrate` binary (local, CI, deploy task) |
| Readonly grants and schema registry | `migrate` binary |
| Seed SQL | `seed` binary (`make seed`) |
| API runtime | Reads and writes through repositories only |

## Consequences

### Positive

- Clear split between schema (migrations), fixtures (seed), and runtime (repositories).
- Applied history never changes, and a person applies every migration.
- No `psql` or sqlx CLI needed on developer machines; the binaries share the API's `DB_*` variables.

### Negative

- Migration SQL is written by hand, so column order and reversibility depend on review (see the `postgres-column-alignment` rule).
- Deploys depend on running `migrate` before the new API version.

### Neutral

- Tests that need Postgres are marked `#[ignore]` and run with `make test-db`.

## Alternatives Considered

| Alternative | Why rejected |
| --- | --- |
| Diesel with schema-diff generation | Closest to a generated workflow, but sync-first, heavier compile times, and the CLI needs `libpq` |
| SeaORM migrations in Rust code | Migrations as code are still hand-written and add an ORM layer the template does not need |
| Run migrations at API startup | Couples runtime to schema changes; unsafe with several instances starting at once |

## References

- [env.md](../env.md)
- [ADR-001: Domain-Driven Design](./001-domain-driven-design.md)
- [sqlx migrations](https://docs.rs/sqlx/latest/sqlx/macro.migrate.html)
