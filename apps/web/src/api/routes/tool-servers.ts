// MCP tool servers (`/tool-servers`). Credentials only go in: answers say whether they are stored
// (`credentials`), never what they are.

import {
  type CreateToolServerRequest,
  type ToolServerResponse,
  type ToolTestResponse,
  toolServerBodySchema,
  toolServerListBodySchema,
  toolTestBodySchema,
  type UpdateToolServerRequest,
} from "@copilot/contracts";
import { apiFetch, apiRequest } from "@web/api/client";
import API from "@web/api/endpoints";

/** `GET /tool-servers`: the caller's servers, oldest first. */
export async function fetchToolServers(signal?: AbortSignal): Promise<ToolServerResponse[]> {
  return (await apiFetch(API.toolServers, toolServerListBodySchema, { signal })).items;
}

/** `GET /tool-servers/{id}`: the server with its cached tools and tool modes. */
export async function fetchToolServer(
  serverId: string,
  signal?: AbortSignal
): Promise<ToolServerResponse> {
  return (await apiFetch(API.toolServer(serverId), toolServerBodySchema, { signal })).data;
}

/** `POST /tool-servers`. The server has no tools until its first test. */
export async function createToolServer(body: CreateToolServerRequest): Promise<ToolServerResponse> {
  return (await apiFetch(API.toolServers, toolServerBodySchema, { method: "POST", body })).data;
}

/**
 * `PATCH /tool-servers/{id}`: changes only the fields `body` carries. `credentials` replaces the
 * stored ones (`null` removes them); `toolModes` replaces every tool's mode.
 */
export async function updateToolServer(
  serverId: string,
  body: UpdateToolServerRequest
): Promise<ToolServerResponse> {
  return (await apiFetch(API.toolServer(serverId), toolServerBodySchema, { method: "PATCH", body }))
    .data;
}

/** `DELETE /tool-servers/{id}`: projects and chats lose the server. */
export async function deleteToolServer(serverId: string): Promise<void> {
  await apiRequest(API.toolServer(serverId), { method: "DELETE" });
}

/** `POST /tool-servers/{id}/test`: connects, lists the tools, and says what changed. */
export async function testToolServer(serverId: string): Promise<ToolTestResponse> {
  return (await apiFetch(API.toolServerTest(serverId), toolTestBodySchema, { method: "POST" }))
    .data;
}

/** `POST /tool-servers/{id}/review`: accepts a changed tool list, so tools follow their modes again. */
export async function reviewToolServer(serverId: string): Promise<ToolServerResponse> {
  return (await apiFetch(API.toolServerReview(serverId), toolServerBodySchema, { method: "POST" }))
    .data;
}
