import type { ToolServerResponse } from "@copilot/contracts";
import { useQuery } from "@tanstack/react-query";
import { keys } from "@web/api/routes/keys";
import { fetchToolServers } from "@web/api/routes/tool-servers";
import { Badge } from "@web/components/Badge/Badge";
import { QueryState } from "@web/components/Settings/QueryState";
import { SettingsPage } from "@web/components/Settings/SettingsPage";
import path, { link } from "@web/routes/path";
import { plural } from "@web/routes/settings/format";
import { LuChevronRight, LuPlus } from "react-icons/lu";
import { Link } from "react-router-dom";

function Row({ server }: { server: ToolServerResponse }) {
  const tools = server.tools === null ? "Not tested yet" : plural(server.tools.length, "tool");
  return (
    <li>
      <Link to={link.toolServer(server.id)} className="list-row">
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-center gap-2 text-sm font-medium">
            <span className="break-words">{server.name}</span>
            {server.enabled ? null : <Badge>Off</Badge>}
            {server.reviewRequired ? <Badge tone="warn">Review changes</Badge> : null}
            {server.approval === "auto" ? <Badge tone="accent">Auto</Badge> : null}
          </p>
          <p className="truncate text-sm text-muted">{`${tools} · ${server.url}`}</p>
        </div>
        <LuChevronRight aria-hidden className="shrink-0 text-muted" />
      </Link>
    </li>
  );
}

/** Settings, Tool servers: the caller's MCP servers. */
export default function ToolServersPage() {
  const servers = useQuery({
    queryKey: keys.toolServers,
    queryFn: ({ signal }) => fetchToolServers(signal),
  });
  const list = servers.data ?? [];

  return (
    <SettingsPage
      title="Tool Servers"
      description="MCP servers whose tools the model can call. Turn them on per project or per chat."
      actions={
        <Link to={path.toolServerNew} className="button-link">
          <LuPlus aria-hidden />
          Add tool server
        </Link>
      }
    >
      <QueryState query={servers}>
        {list.length === 0 ? (
          <div className="card-panel text-center">
            <p className="text-sm font-medium">No tool servers yet</p>
            <p className="mt-1 text-sm text-muted">
              Add a remote MCP server by its address, with a token or headers when it needs them.
            </p>
          </div>
        ) : (
          <ul className="list-panel">
            {list.map((server) => (
              <Row key={server.id} server={server} />
            ))}
          </ul>
        )}
      </QueryState>
    </SettingsPage>
  );
}
