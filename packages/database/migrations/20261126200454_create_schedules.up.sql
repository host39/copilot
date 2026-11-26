-- create_schedules: the `schedule` domain (docs/design/copilot.md, Data Model). Columns follow
-- .ai/rules/postgres-column-alignment.md.

-- Prompts that run on a cron rule, each firing in a new chat. `next_run_at` is null while
-- disabled. `source_chat_id` (the chat whose assistant created the schedule) references
-- `chats`, which does not exist yet: create_chats adds that foreign key.
CREATE TABLE schedules (
  created_at           timestamptz NOT NULL,
  updated_at           timestamptz NOT NULL,
  next_run_at          timestamptz,
  last_run_at          timestamptz,
  consecutive_failures integer     NOT NULL DEFAULT 0 CHECK (consecutive_failures >= 0),
  id                   uuid        PRIMARY KEY,
  user_id              uuid        NOT NULL REFERENCES auth.users (id) ON DELETE CASCADE,
  project_id           uuid,
  model_id             uuid,
  source_chat_id       uuid,
  enabled              boolean     NOT NULL DEFAULT true,
  name                 text        NOT NULL,
  cron                 text        NOT NULL,
  timezone             text        NOT NULL,
  prompt               text        NOT NULL,
  instructions         text        NOT NULL DEFAULT '',
  reasoning            text        CHECK (reasoning IN ('off', 'low', 'medium', 'high')),
  on_ask               text        NOT NULL DEFAULT 'deny' CHECK (on_ask IN ('deny', 'pause')),
  created_by           text        NOT NULL CHECK (created_by IN ('user', 'assistant')),
  last_status          text        CHECK (last_status IN ('ok', 'error', 'awaiting_approval')),
  last_error           text,
  disabled_reason      text,
  UNIQUE (id, user_id),
  CHECK (enabled OR next_run_at IS NULL),
  FOREIGN KEY (project_id, user_id) REFERENCES projects (id, user_id) ON DELETE CASCADE,
  FOREIGN KEY (model_id, user_id) REFERENCES models (id, user_id) ON DELETE SET NULL (model_id)
);

-- The scheduler's tick: enabled schedules by due time.
CREATE INDEX schedules_next_run_at_idx ON schedules (next_run_at) WHERE enabled;
CREATE INDEX schedules_user_id_idx ON schedules (user_id);
CREATE INDEX schedules_project_id_idx ON schedules (project_id);
CREATE INDEX schedules_model_id_idx ON schedules (model_id);
CREATE INDEX schedules_source_chat_id_idx ON schedules (source_chat_id);
