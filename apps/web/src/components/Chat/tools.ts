// The tools menu's rules: which tool servers and built-in tools a chat uses (the project's, with
// the chat's own choices on top), what a change stores, and when the tools button warns. Pure
// functions, so tests check every rule without a browser.

import type {
  BuiltinTool,
  Capabilities,
  ProjectResponse,
  ToolOverride,
  ToolServerResponse,
} from "@copilot/contracts";

/** A tool server row of the menu. */
export type ServerRow = {
  id: string;
  name: string;
  on: boolean;
  /** The project turns it on for its chats ("from project"). */
  fromProject: boolean;
  /** Its tool list changed: every call asks until it is reviewed in Settings. */
  review: boolean;
};

type ProjectTools = Pick<ProjectResponse, "toolServerIds" | "builtinTools">;

/**
 * The servers the menu lists (turned on in Settings), each on when the project turns it on and
 * the chat does not turn it off, or the chat turns it on.
 */
export function serverRows(
  servers: ToolServerResponse[] | undefined,
  project: ProjectTools | null | undefined,
  overrides: ToolOverride[]
): ServerRow[] {
  const fromProject = new Set(project?.toolServerIds ?? []);
  const chosen = new Map(overrides.map((item) => [item.serverId, item.enabled]));
  return (servers ?? [])
    .filter((server) => server.enabled)
    .map((server) => ({
      id: server.id,
      name: server.name,
      on: chosen.get(server.id) ?? fromProject.has(server.id),
      fromProject: fromProject.has(server.id),
      review: server.reviewRequired,
    }));
}

/**
 * The chat's overrides after turning `serverId` on or off: an override only where the chat
 * differs from its project, so following the project again stores nothing.
 */
export function setServer(
  project: ProjectTools | null | undefined,
  overrides: ToolOverride[],
  serverId: string,
  on: boolean
): ToolOverride[] {
  const rest = overrides.filter((item) => item.serverId !== serverId);
  const inherited = project?.toolServerIds.includes(serverId) ?? false;
  return inherited === on ? rest : [...rest, { serverId, enabled: on }];
}

/** The built-in tools a chat uses: its own list, else its project's (`null` follows the project). */
export function builtinOn(
  own: BuiltinTool[] | null,
  project: ProjectTools | null | undefined
): BuiltinTool[] {
  return own ?? project?.builtinTools ?? [];
}

/** The chat's built-in tools after turning `tool` on or off. */
export function setBuiltin(current: BuiltinTool[], tool: BuiltinTool, on: boolean): BuiltinTool[] {
  const rest = current.filter((item) => item !== tool);
  return on ? [...rest, tool] : rest;
}

/** The tools button warns when tools are on but the model cannot call them. */
export function toolsWarning(
  on: boolean,
  capabilities: Pick<Capabilities, "tools"> | null | undefined
): boolean {
  return on && capabilities !== null && capabilities !== undefined && !capabilities.tools;
}

/** The line above the composer while the tools button warns. */
export const TOOLS_OFF_NOTE = "Tools are off for this model: it cannot call them.";
