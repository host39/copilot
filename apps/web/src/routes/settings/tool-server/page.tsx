import { AddToolServer } from "@web/routes/settings/tool-server/add";
import { ToolServerDetails } from "@web/routes/settings/tool-server/details";
import { useParams } from "react-router-dom";

/** `/settings/tools/new` adds a server; `/settings/tools/:serverId` shows one. */
export default function ToolServerPage() {
  const { serverId } = useParams();
  return serverId ? <ToolServerDetails key={serverId} id={serverId} /> : <AddToolServer />;
}
