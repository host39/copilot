-- create_projects: the `project` domain, projects and their tool servers (docs/design/copilot.md,
-- Data Model). Columns follow .ai/rules/postgres-column-alignment.md.

-- Reusable chat contexts. A nullable reference uses `ON DELETE SET NULL (<column>)`, so deleting
-- a model clears `default_model_id` and never `user_id`.
CREATE TABLE projects (
  created_at       timestamptz NOT NULL,
  updated_at       timestamptz NOT NULL,
  id               uuid        PRIMARY KEY,
  user_id          uuid        NOT NULL REFERENCES auth.users (id) ON DELETE CASCADE,
  default_model_id uuid,
  pinned           boolean     NOT NULL DEFAULT false,
  name             text        NOT NULL,
  icon             text        NOT NULL DEFAULT '',
  instructions     text        NOT NULL DEFAULT '',
  reasoning        text        CHECK (reasoning IN ('off', 'low', 'medium', 'high')),
  builtin_tools    text[]      NOT NULL DEFAULT '{}'
    CHECK (builtin_tools <@ ARRAY['schedules']),
  UNIQUE (id, user_id),
  FOREIGN KEY (default_model_id, user_id) REFERENCES models (id, user_id)
    ON DELETE SET NULL (default_model_id)
);

CREATE INDEX projects_user_id_idx ON projects (user_id);
CREATE INDEX projects_default_model_id_idx ON projects (default_model_id);

-- The tool servers a project enables, saved with the project in one transaction.
CREATE TABLE project_tool_servers (
  project_id     uuid NOT NULL,
  tool_server_id uuid NOT NULL,
  user_id        uuid NOT NULL,
  PRIMARY KEY (project_id, tool_server_id),
  FOREIGN KEY (project_id, user_id) REFERENCES projects (id, user_id) ON DELETE CASCADE,
  FOREIGN KEY (tool_server_id, user_id) REFERENCES tool_servers (id, user_id) ON DELETE CASCADE
);

CREATE INDEX project_tool_servers_tool_server_id_idx ON project_tool_servers (tool_server_id);
