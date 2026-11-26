-- create_providers: the `provider` domain, connections and their models (docs/design/copilot.md,
-- Data Model). Columns follow .ai/rules/postgres-column-alignment.md.
--
-- Tenant isolation: every owner-scoped table has `user_id`, a referenced table has
-- `UNIQUE (id, user_id)`, and every reference between owner-scoped rows is a composite foreign
-- key on (id, user_id), so a row can never point at another user's row.

-- A user's connection to a model provider. `api_key_sealed` is AES-256-GCM sealed and bound to
-- the row; the readonly role never reads it.
CREATE TABLE providers (
  created_at     timestamptz NOT NULL,
  updated_at     timestamptz NOT NULL,
  id             uuid        PRIMARY KEY,
  user_id        uuid        NOT NULL REFERENCES auth.users (id) ON DELETE CASCADE,
  enabled        boolean     NOT NULL DEFAULT true,
  kind           text        NOT NULL CHECK (kind IN ('openai', 'anthropic')),
  preset         text        NOT NULL
    CHECK (preset IN ('openai', 'anthropic', 'gemini', 'openrouter', 'ollama', 'custom')),
  name           text        NOT NULL,
  base_url       text        NOT NULL,
  api_key_sealed bytea,
  UNIQUE (id, user_id)
);

CREATE INDEX providers_user_id_idx ON providers (user_id);

-- Models a refresh found. A refresh never deletes one: a model the provider no longer lists
-- becomes unavailable. `capabilities` is the detected `CapabilitiesRecord`; user overrides live
-- apart, so a refresh never touches them.
CREATE TABLE models (
  created_at           timestamptz NOT NULL,
  updated_at           timestamptz NOT NULL,
  context_window       integer     CHECK (context_window >= 0),
  max_output_tokens    integer     CHECK (max_output_tokens >= 0),
  id                   uuid        PRIMARY KEY,
  user_id              uuid        NOT NULL,
  provider_id          uuid        NOT NULL,
  enabled              boolean     NOT NULL DEFAULT false,
  available            boolean     NOT NULL DEFAULT true,
  remote_id            text        NOT NULL,
  display_name         text        NOT NULL,
  capabilities         jsonb       NOT NULL,
  capability_overrides jsonb       NOT NULL DEFAULT '{}',
  UNIQUE (id, user_id),
  UNIQUE (provider_id, remote_id),
  FOREIGN KEY (provider_id, user_id) REFERENCES providers (id, user_id) ON DELETE CASCADE
);

CREATE INDEX models_user_id_idx ON models (user_id);
