# Environment Seed SQL

| Folder | Used by |
| --- | --- |
| `local/` | `make seed`: all `*.sql` files in byte-wise filename order |
| `dev/` | `cargo run -p copilot-database --bin seed -- dev` against a shared development database |

Schema and platform invariants belong in migrations (`../migrations/`), not in seed folders. Keep seed files idempotent (`INSERT ... ON CONFLICT DO NOTHING`) so they can run more than once.

The `seed` binary refuses to run when `ENVIRONMENT=production`.

Optional skip: set `SEED_SKIP_IF_POPULATED=1` and `SEED_SKIP_QUERY` to a SQL expression that returns `true` when seed should be skipped (for example `SELECT EXISTS (SELECT 1 FROM widgets)`).
