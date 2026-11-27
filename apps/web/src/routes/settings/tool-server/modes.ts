// A tool server's tool list in Settings: each tool's mode (`off`, `ask`, `auto`; the server's
// approval mode when none is set), the draft the user edits, and what a test changed.

import type { ServerTool, ToolChanges, ToolMode, ToolServerResponse } from "@copilot/contracts";
import { plural } from "@web/routes/settings/format";

/** The mode a tool runs with: its own, else the server's approval mode. */
export function modeFor(server: ToolServerResponse, tool: string): ToolMode {
  return server.toolModes[tool] ?? server.approval;
}

/**
 * The `toolModes` body for the listed tools, with the draft's choices applied. The server refuses
 * modes for tools it does not list, so only listed tools are sent.
 */
export function modesBody(
  server: ToolServerResponse,
  draft: Readonly<Record<string, ToolMode>>
): Record<string, ToolMode> {
  const body: Record<string, ToolMode> = {};
  for (const tool of server.tools ?? [])
    body[tool.name] = draft[tool.name] ?? modeFor(server, tool.name);
  return body;
}

/** Whether the draft changes any listed tool's mode. */
export function modesChanged(
  server: ToolServerResponse,
  draft: Readonly<Record<string, ToolMode>>
): boolean {
  return (server.tools ?? []).some(
    (tool) => draft[tool.name] !== undefined && draft[tool.name] !== modeFor(server, tool.name)
  );
}

/** Tools by title (or name), so the list reads alphabetically. */
export function sortTools(tools: readonly ServerTool[]): ServerTool[] {
  return [...tools].sort((a, b) =>
    (a.title ?? a.name).localeCompare(b.title ?? b.name, "en", { sensitivity: "base" })
  );
}

function named(count: number, verb: string, names: readonly string[]): string {
  return `${count} ${verb} (${names.join(", ")})`;
}

/** What a test found ("Found 4 tools. 1 added (search), 1 changed (fetch)."). */
export function testSummary(tools: number, changes: ToolChanges): string {
  const parts = [
    changes.added.length > 0 ? named(changes.added.length, "added", changes.added) : "",
    changes.removed.length > 0 ? named(changes.removed.length, "removed", changes.removed) : "",
    changes.changed.length > 0 ? named(changes.changed.length, "changed", changes.changed) : "",
  ].filter(Boolean);
  const found = `Found ${plural(tools, "tool")}.`;
  return parts.length === 0 ? found : `${found} ${parts.join(", ")}.`;
}
