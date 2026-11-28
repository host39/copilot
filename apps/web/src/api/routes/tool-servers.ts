// MCP tool servers (`/tool-servers`). Credentials only go in: answers say whether they are stored
// (`credentials`) and how an OAuth sign-in stands (`oauth`), never what they are. A stdio server's
// environment answers with its variable names only.

import {
  type CapabilitiesResponse,
  type CreateToolServerRequest,
  capabilitiesBodySchema,
  type OAuthConfirmRequest,
  type OAuthStartResponse,
  oauthStartBodySchema,
  type ToolServerResponse,
  type ToolTestResponse,
  toolServerBodySchema,
  toolServerListBodySchema,
  toolTestBodySchema,
  type UpdateToolServerRequest,
} from "@copilot/contracts";
import { apiFetch, apiRequest } from "@web/api/client";
import API from "@web/api/endpoints";

/**
 * `GET /capabilities`: what the server lets the caller do (`stdioServers`: an administrator on a
 * server with `MCP_STDIO_ENABLED`).
 */
export async function fetchCapabilities(signal?: AbortSignal): Promise<CapabilitiesResponse> {
  return (await apiFetch(API.capabilities, capabilitiesBodySchema, { signal })).data;
}

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

/**
 * `POST /tool-servers/{id}/oauth/start`: the authorization URL to open in the browser. The
 * sign-in comes back to the server, which then sends the browser to `/oauth/done`.
 */
export async function startSignIn(serverId: string): Promise<OAuthStartResponse> {
  return (await apiFetch(API.toolServerSignIn(serverId), oauthStartBodySchema, { method: "POST" }))
    .data;
}

/**
 * `POST /tool-servers/{id}/oauth/confirm`: finishes a sign-in that came back, with the code the
 * returning browser showed. Only the server's owner can; anyone else gets `404`.
 */
export async function confirmSignIn(serverId: string, code: string): Promise<ToolServerResponse> {
  const body: OAuthConfirmRequest = { code };
  return (
    await apiFetch(API.toolServerConfirm(serverId), toolServerBodySchema, { method: "POST", body })
  ).data;
}

/** `DELETE /tool-servers/{id}/oauth`: forgets the server's sign-in. */
export async function signOut(serverId: string): Promise<ToolServerResponse> {
  return (
    await apiFetch(API.toolServerSignOut(serverId), toolServerBodySchema, { method: "DELETE" })
  ).data;
}
