-- create_tool_servers: the `tool` domain, remote MCP servers (docs/design/copilot.md, Data
-- Model). Columns follow .ai/rules/postgres-column-alignment.md.

-- A user's MCP server. `headers_sealed` holds the sealed JSON map of the bearer token and custom
-- headers; the readonly role never reads it. `slug` is immutable (repositories never update it).
-- `tools_cached` (the last `tools/list` result with `ttlMs`) and `tools_fetched_at` are set
-- together.
CREATE TABLE tool_servers (
  created_at            timestamptz NOT NULL,
  updated_at            timestamptz NOT NULL,
  tools_fetched_at      timestamptz,
  id                    uuid        PRIMARY KEY,
  user_id               uuid        NOT NULL REFERENCES auth.users (id) ON DELETE CASCADE,
  enabled               boolean     NOT NULL DEFAULT true,
  tools_review_required boolean     NOT NULL DEFAULT false,
  name                  text        NOT NULL,
  slug                  text        NOT NULL CHECK (slug ~ '^[a-z0-9]{1,12}$'),
  url                   text        NOT NULL,
  approval              text        NOT NULL DEFAULT 'ask' CHECK (approval IN ('ask', 'auto')),
  headers_sealed        bytea,
  protocol_version      text,
  tool_modes            jsonb       NOT NULL DEFAULT '{}',
  tools_cached          jsonb,
  UNIQUE (id, user_id),
  UNIQUE (user_id, slug),
  CHECK ((tools_cached IS NULL) = (tools_fetched_at IS NULL))
);
