-- Local development database initialization.
-- Runs once when the Postgres container is first created (see docker-compose.yml).
--
-- The database itself (POSTGRES_DB) is created by the postgres image before this
-- script runs, so this is where you add extensions, roles, or schemas your app
-- needs. Keep it idempotent where possible.

-- Example: enable a commonly used extension.
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
