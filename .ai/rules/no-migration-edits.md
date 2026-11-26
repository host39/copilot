# Migrations (Committed Files Are Read-Only for AI)

Migrations in `packages/database/migrations/` are plain SQL applied by sqlx. sqlx stores a checksum for every applied migration and refuses to run when a committed file changes, so committed migrations are append-only.

## Workflow

1. Scaffold: `make migrations name=<snake_case_name>` creates `<timestamp>_<name>.up.sql` and `<timestamp>_<name>.down.sql`.
2. Write the SQL in the new files. Follow the `postgres-column-alignment` rule; keep `.down.sql` an exact revert of `.up.sql`.
3. Implement repository changes in `packages/database/src/repositories/` and register new tables in `packages/database/src/operations/describe/mod.rs` so readonly grants stay in sync.
4. **Stop**: tell the developer to review the migration.
5. The developer runs `make migrate` (and `make test-db` when tests need the new schema).

## Allowed

- **Read** any migration file with the Read or Glob tools.
- Run `make migrations name=<name>` and write SQL in migration files that are **not committed yet**.
- Edit seed files under `packages/database/seed/` (seed data is separate from migrations).

## Not Allowed

- Modifying, renaming, or deleting a migration that exists in the last commit. Fix mistakes with a new migration.
- Applying or reverting migrations: `make migrate`, `cargo run -p copilot-database --bin migrate`, `sqlx migrate run` or `revert`.
- Shell commands against `packages/database/migrations/`; use the file tools.

`.ai/hooks/deny-migration-edits.py` enforces this for Claude Code and Cursor.

## Why Not Generated Migrations

With Drizzle, the schema lives in TypeScript and migrations are generated from it, so assistants never touch SQL. sqlx has no schema DSL: the migration SQL is the schema change. This rule keeps the same safety property (applied history never changes, and a person applies every migration) while letting assistants draft new migrations.
