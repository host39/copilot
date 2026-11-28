-- confirm_tool_server_sign_in: the P7 review's sign-in hardening (docs/design/copilot.md, Tool
-- Server Sign-In). Columns are appended, which .ai/rules/postgres-column-alignment.md accepts for
-- small additions.

-- The authorization server a pre-registered client belongs to: set by the client's first sign-in
-- and checked by every later one, so the client's secret never reaches another token endpoint.
-- Cleared when the client is saved again or forgotten.
ALTER TABLE tool_servers
  ADD COLUMN oauth_client_issuer text,
  ADD CONSTRAINT tool_servers_oauth_client_issuer_check
    CHECK (oauth_client_issuer IS NULL OR oauth_client_id IS NOT NULL);

-- A flow whose callback came back holds the exchanged tokens (sealed) until the server's owner
-- confirms them with the code the returning browser was shown: `confirm_hash` is that code's
-- SHA-256, `attempts` counts the tries. Waiting flows have neither.
ALTER TABLE oauth_flows
  ADD COLUMN attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
  ADD COLUMN confirm_hash bytea CHECK (confirm_hash IS NULL OR octet_length(confirm_hash) = 32);
