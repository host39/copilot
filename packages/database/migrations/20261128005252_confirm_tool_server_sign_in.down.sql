-- Reverts confirm_tool_server_sign_in. Held sign-ins are dropped with their tokens.
DELETE FROM oauth_flows WHERE confirm_hash IS NOT NULL;

ALTER TABLE oauth_flows
  DROP COLUMN confirm_hash,
  DROP COLUMN attempts;

ALTER TABLE tool_servers
  DROP CONSTRAINT tool_servers_oauth_client_issuer_check,
  DROP COLUMN oauth_client_issuer;
