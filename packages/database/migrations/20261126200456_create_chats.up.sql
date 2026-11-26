-- create_chats: the `chat` domain, chats with their tool overrides and approvals, messages, and
-- turn snapshots (docs/design/copilot.md, Data Model). Columns follow
-- .ai/rules/postgres-column-alignment.md.

-- A conversation with one model, in a project or standalone. `updated_at` moves with every
-- message write (not with settings changes) and orders the chat list.
CREATE TABLE chats (
  created_at    timestamptz NOT NULL,
  updated_at    timestamptz NOT NULL,
  last_read_at  timestamptz,
  id            uuid        PRIMARY KEY,
  user_id       uuid        NOT NULL REFERENCES auth.users (id) ON DELETE CASCADE,
  project_id    uuid,
  model_id      uuid,
  schedule_id   uuid,
  pinned        boolean     NOT NULL DEFAULT false,
  voice_replies boolean     NOT NULL DEFAULT false,
  title         text        NOT NULL,
  instructions  text        NOT NULL DEFAULT '',
  reasoning     text        CHECK (reasoning IN ('off', 'low', 'medium', 'high')),
  builtin_tools text[]      CHECK (builtin_tools <@ ARRAY['schedules']),
  UNIQUE (id, user_id),
  FOREIGN KEY (project_id, user_id) REFERENCES projects (id, user_id) ON DELETE CASCADE,
  FOREIGN KEY (model_id, user_id) REFERENCES models (id, user_id) ON DELETE SET NULL (model_id),
  FOREIGN KEY (schedule_id, user_id) REFERENCES schedules (id, user_id)
    ON DELETE SET NULL (schedule_id)
);

CREATE INDEX chats_user_id_updated_at_idx ON chats (user_id, updated_at DESC);
CREATE INDEX chats_project_id_idx ON chats (project_id);
CREATE INDEX chats_model_id_idx ON chats (model_id);
CREATE INDEX chats_schedule_id_idx ON chats (schedule_id);

-- The schedule's reference to the chat it was created in, now that `chats` exists.
ALTER TABLE schedules
  ADD CONSTRAINT schedules_source_chat_id_fkey
  FOREIGN KEY (source_chat_id, user_id) REFERENCES chats (id, user_id)
  ON DELETE SET NULL (source_chat_id);

-- Per-chat tool server overrides: `enabled` true adds a server, false removes a project server
-- in this chat.
CREATE TABLE chat_tool_servers (
  chat_id        uuid    NOT NULL,
  tool_server_id uuid    NOT NULL,
  user_id        uuid    NOT NULL,
  enabled        boolean NOT NULL,
  PRIMARY KEY (chat_id, tool_server_id),
  FOREIGN KEY (chat_id, user_id) REFERENCES chats (id, user_id) ON DELETE CASCADE,
  FOREIGN KEY (tool_server_id, user_id) REFERENCES tool_servers (id, user_id) ON DELETE CASCADE
);

CREATE INDEX chat_tool_servers_tool_server_id_idx ON chat_tool_servers (tool_server_id);

-- "Approve for this chat" decisions, by server and original tool name (never by alias).
CREATE TABLE chat_tool_approvals (
  chat_id        uuid NOT NULL,
  tool_server_id uuid NOT NULL,
  user_id        uuid NOT NULL,
  tool_name      text NOT NULL,
  PRIMARY KEY (chat_id, tool_server_id, tool_name),
  FOREIGN KEY (chat_id, user_id) REFERENCES chats (id, user_id) ON DELETE CASCADE,
  FOREIGN KEY (tool_server_id, user_id) REFERENCES tool_servers (id, user_id) ON DELETE CASCADE
);

CREATE INDEX chat_tool_approvals_tool_server_id_idx ON chat_tool_approvals (tool_server_id);

-- Messages, ordered by `seq`. `parts` is a `PartRecord` array, `provider_state` is
-- provider-opaque output (never sent to clients), and `text_content` joins the text parts for
-- search. `context_hash` is set on assistant messages only.
CREATE TABLE messages (
  seq            bigint      GENERATED ALWAYS AS IDENTITY,
  created_at     timestamptz NOT NULL,
  updated_at     timestamptz NOT NULL,
  id             uuid        PRIMARY KEY,
  user_id        uuid        NOT NULL,
  chat_id        uuid        NOT NULL,
  model_id       uuid,
  provider_id    uuid,
  role           text        NOT NULL CHECK (role IN ('user', 'assistant', 'tool')),
  status         text        NOT NULL CHECK (
    status IN ('streaming', 'complete', 'awaiting_approval', 'cancelled', 'error', 'refused')
  ),
  finish_reason  text        CHECK (
    finish_reason IN ('stop', 'tool_calls', 'length', 'refusal', 'error', 'other', 'tool_limit')
  ),
  context_hash   bytea       CHECK (octet_length(context_hash) = 32),
  parts          jsonb       NOT NULL,
  provider_state jsonb,
  usage          jsonb,
  error          text,
  text_content   text        NOT NULL DEFAULT '',
  CHECK (context_hash IS NULL OR role = 'assistant'),
  FOREIGN KEY (chat_id, user_id) REFERENCES chats (id, user_id) ON DELETE CASCADE,
  FOREIGN KEY (model_id, user_id) REFERENCES models (id, user_id) ON DELETE SET NULL (model_id),
  FOREIGN KEY (provider_id, user_id) REFERENCES providers (id, user_id)
    ON DELETE SET NULL (provider_id)
);

CREATE INDEX messages_chat_id_seq_idx ON messages (chat_id, seq);
-- Open turns: unread approval flags per chat, and `recover` at startup.
CREATE INDEX messages_open_idx ON messages (chat_id, status)
  WHERE status IN ('streaming', 'awaiting_approval');
CREATE INDEX messages_text_content_idx ON messages
  USING gin (to_tsvector('simple', text_content));
CREATE INDEX messages_model_id_idx ON messages (model_id);
CREATE INDEX messages_provider_id_idx ON messages (provider_id);

-- Turn snapshots: what the model saw (system prompt, tools, file preamble, window start,
-- reasoning level), keyed by chat and context hash, so continuations and replays use the
-- same context.
CREATE TABLE turn_contexts (
  created_at      timestamptz NOT NULL,
  chat_id         uuid        NOT NULL,
  user_id         uuid        NOT NULL,
  window_start_id uuid,
  reasoning       text        CHECK (reasoning IN ('off', 'low', 'medium', 'high')),
  hash            bytea       NOT NULL CHECK (octet_length(hash) = 32),
  system          text        NOT NULL,
  tools           jsonb       NOT NULL,
  files           jsonb       NOT NULL,
  PRIMARY KEY (chat_id, hash),
  FOREIGN KEY (chat_id, user_id) REFERENCES chats (id, user_id) ON DELETE CASCADE
);
