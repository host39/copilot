-- add_stdio_tool_servers: revert everything the .up.sql file does. stdio servers have no URL, so
-- they are deleted first (with their project and chat selections, by the cascades).
DELETE FROM tool_servers WHERE transport = 'stdio';

ALTER TABLE tool_servers
  DROP CONSTRAINT tool_servers_transport_check_columns,
  DROP COLUMN args,
  DROP COLUMN cwd,
  DROP COLUMN env_sealed,
  DROP COLUMN command,
  DROP COLUMN transport,
  ALTER COLUMN url SET NOT NULL;
