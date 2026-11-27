-- add_tool_server_sign_in: OAuth sign-in to remote MCP servers (docs/design/copilot.md, Tool
-- Server Sign-In, and Data Model). Columns follow .ai/rules/postgres-column-alignment.md.

-- How a server's requests authenticate: no credentials, the sealed bearer token and headers
-- (`headers_sealed`), or an OAuth sign-in. `oauth_client_id` and `oauth_client_secret_sealed` are
-- a pre-registered client the user entered; `oauth_tokens_sealed` holds the sealed access and
-- refresh tokens with what a refresh needs (token endpoint, client, resource), bound to this row;
-- `oauth_expires_at` and `oauth_refreshable` let the settings show the sign-in's state without
-- opening it. Headers exist only in `headers` mode and the OAuth columns only in `oauth` mode.
ALTER TABLE tool_servers
  ADD COLUMN oauth_expires_at timestamptz,
  ADD COLUMN oauth_refreshable boolean NOT NULL DEFAULT false,
  ADD COLUMN auth_mode text NOT NULL DEFAULT 'none'
    CHECK (auth_mode IN ('none', 'headers', 'oauth')),
  ADD COLUMN oauth_client_id text,
  ADD COLUMN oauth_client_secret_sealed bytea,
  ADD COLUMN oauth_tokens_sealed bytea;

UPDATE tool_servers SET auth_mode = 'headers' WHERE headers_sealed IS NOT NULL;

ALTER TABLE tool_servers
  ADD CONSTRAINT tool_servers_auth_mode_check_columns CHECK (
    (headers_sealed IS NULL OR auth_mode = 'headers')
    AND (auth_mode = 'oauth' OR (
      oauth_client_id IS NULL AND oauth_client_secret_sealed IS NULL
      AND oauth_tokens_sealed IS NULL
    ))
    AND (oauth_client_secret_sealed IS NULL OR oauth_client_id IS NOT NULL)
    AND (oauth_tokens_sealed IS NOT NULL OR (oauth_expires_at IS NULL AND NOT oauth_refreshable))
  );

-- A sign-in in progress: started by `POST /api/tool-servers/{id}/oauth/start` and used up by
-- the first callback that names its `state`. Only the SHA-256 of the `state` is stored; `sealed`
-- holds the PKCE verifier, the issuer the callback must name, and the token request's other
-- parts, bound to this row. A server keeps at most one; rows past `expires_at` are deleted when
-- the next sign-in starts.
CREATE TABLE oauth_flows (
  created_at     timestamptz NOT NULL,
  expires_at     timestamptz NOT NULL,
  id             uuid        PRIMARY KEY,
  user_id        uuid        NOT NULL,
  tool_server_id uuid        NOT NULL,
  state_hash     bytea       NOT NULL UNIQUE CHECK (octet_length(state_hash) = 32),
  sealed         bytea       NOT NULL,
  UNIQUE (tool_server_id),
  FOREIGN KEY (tool_server_id, user_id) REFERENCES tool_servers (id, user_id) ON DELETE CASCADE
);

CREATE INDEX oauth_flows_expires_at_idx ON oauth_flows (expires_at);
