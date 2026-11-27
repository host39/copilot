-- add_tool_server_sign_in: revert everything the .up.sql file does.
DROP TABLE oauth_flows;

ALTER TABLE tool_servers
  DROP CONSTRAINT tool_servers_auth_mode_check_columns,
  DROP COLUMN oauth_tokens_sealed,
  DROP COLUMN oauth_client_secret_sealed,
  DROP COLUMN oauth_client_id,
  DROP COLUMN auth_mode,
  DROP COLUMN oauth_refreshable,
  DROP COLUMN oauth_expires_at;
