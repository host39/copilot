# PostgreSQL Column Alignment

PostgreSQL pads fixed-width columns to their alignment boundary, so mixing widths wastes space in every row. Order columns in each `CREATE TABLE` in a new migration (see the `no-migration-edits` rule for which files you may write).

## Rule

Within each `CREATE TABLE`, list columns in this order:

1. **8-byte**: `timestamptz`, `timestamp`, `bigint`, `bigserial`, `double precision`
2. **4-byte**: `integer`, `serial`, `real`, `date`, enum types
3. **16-byte**: `uuid`
4. **1-byte**: `boolean`
5. **Variable-length last**: `text`, `varchar`, `numeric`, `bytea`, `json`, `jsonb`, arrays

When a migration creates a table, sort the whole column list. `ALTER TABLE ... ADD COLUMN` always appends at the end; that is acceptable for small additions, and a rebuild to restore alignment is a deliberate migration for the developer to approve.

The Rust row structs (`sqlx::FromRow`) map columns by name, so their field order does not need to match.

## Example

```sql
CREATE TABLE widgets (
  created_at   timestamptz NOT NULL,
  updated_at   timestamptz NOT NULL,
  position     integer     NOT NULL,
  status       widget_status NOT NULL,
  id           uuid        PRIMARY KEY,
  owner_id     uuid        NOT NULL,
  archived     boolean     NOT NULL DEFAULT false,
  name         text        NOT NULL,
  settings     jsonb       NOT NULL DEFAULT '{}'
);
```
