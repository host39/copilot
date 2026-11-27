-- add_stdio_tool_servers: tool servers that run a program on the API host and speak MCP over its
-- stdin and stdout (docs/design/copilot.md, stdio Servers, and Data Model). Columns follow
-- .ai/rules/postgres-column-alignment.md (`ALTER TABLE ... ADD COLUMN` appends).

-- `transport` is set when a server is added and never changes. A Streamable HTTP server has a
-- `url`; a stdio server has none, and instead its program's absolute path (`command`), its
-- arguments (`args`, passed as they are, never through a shell), the sealed JSON object of the
-- only environment variables it gets (`env_sealed`, bound to this row), and its working directory
-- relative to `MCP_STDIO_DIR` (`cwd`). A stdio server takes no credentials (`auth_mode` `none`,
-- so no headers or OAuth columns either): its program reads them from its environment.
ALTER TABLE tool_servers
  ALTER COLUMN url DROP NOT NULL,
  ADD COLUMN transport text NOT NULL DEFAULT 'http' CHECK (transport IN ('http', 'stdio')),
  ADD COLUMN command text,
  ADD COLUMN env_sealed bytea,
  ADD COLUMN cwd text,
  ADD COLUMN args text[];

ALTER TABLE tool_servers
  ADD CONSTRAINT tool_servers_transport_check_columns CHECK (
    (transport = 'http' AND url IS NOT NULL AND command IS NULL AND args IS NULL
      AND env_sealed IS NULL AND cwd IS NULL)
    OR (transport = 'stdio' AND url IS NULL AND command IS NOT NULL AND args IS NOT NULL
      AND cwd IS NOT NULL AND auth_mode = 'none')
  );
