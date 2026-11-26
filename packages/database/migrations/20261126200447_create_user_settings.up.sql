-- create_user_settings: the `settings` domain (docs/design/copilot.md, Data Model).
-- Columns follow .ai/rules/postgres-column-alignment.md.

-- Personal instructions, one row per user, written on the first save. A missing row reads as
-- empty instructions, so registration (lib-rustauth's) needs no hook into Copilot.
CREATE TABLE user_settings (
  created_at   timestamptz NOT NULL,
  updated_at   timestamptz NOT NULL,
  user_id      uuid        PRIMARY KEY REFERENCES auth.users (id) ON DELETE CASCADE,
  instructions text        NOT NULL DEFAULT ''
);
