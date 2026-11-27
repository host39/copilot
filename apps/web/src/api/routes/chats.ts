// Chats and their turns. `sendMessage` answers with the turn's server-sent events, each a
// `ChatStreamEvent`, ending with `done`.

import {
  type ChatListQuery,
  type ChatResponse,
  type ChatStreamEvent,
  type ChatSummaryResponse,
  type CreateChatRequest,
  chatBodySchema,
  chatPageBodySchema,
  chatStreamEventSchema,
  type MessageResponse,
  messageListBodySchema,
  type PageResponse,
  type ProjectResponse,
  projectBodySchema,
  type SendMessageRequest,
  type UpdateChatRequest,
} from "@copilot/contracts";
import { apiFetch, apiRequest, apiStream } from "@web/api/client";
import API from "@web/api/endpoints";

/**
 * `GET /chats`: one page, most recently updated first. Pass the page's `next` back as `cursor`
 * for the following one.
 */
export async function fetchChats(
  query: ChatListQuery = {},
  signal?: AbortSignal
): Promise<PageResponse<ChatSummaryResponse>> {
  return apiFetch(API.chats, chatPageBodySchema, { query, signal });
}

/** `POST /chats`: the new chat, created with its first message. */
export async function createChat(body: CreateChatRequest): Promise<ChatResponse> {
  return (await apiFetch(API.chats, chatBodySchema, { method: "POST", body })).data;
}

/** `GET /chats/{id}`. */
export async function fetchChat(chatId: string, signal?: AbortSignal): Promise<ChatResponse> {
  return (await apiFetch(API.chat(chatId), chatBodySchema, { signal })).data;
}

/**
 * `PATCH /chats/{id}`: changes only the fields `body` carries; `null` clears one (a `projectId`
 * of `null` moves the chat out of its project).
 */
export async function updateChat(chatId: string, body: UpdateChatRequest): Promise<ChatResponse> {
  return (await apiFetch(API.chat(chatId), chatBodySchema, { method: "PATCH", body })).data;
}

/** `DELETE /chats/{id}`: the chat with its messages and attachments. */
export async function deleteChat(chatId: string): Promise<void> {
  await apiRequest(API.chat(chatId), { method: "DELETE" });
}

/** `POST /chats/{id}/read`: clears the unread dot. */
export async function markRead(chatId: string): Promise<void> {
  await apiRequest(API.chatRead(chatId), { method: "POST" });
}

/**
 * `POST /chats/{id}/save-as-project`: a project made from a standalone chat (its title,
 * instructions, model, reasoning, tools, and attachments), which the chat moves into.
 */
export async function saveAsProject(chatId: string): Promise<ProjectResponse> {
  return (await apiFetch(API.chatSaveAsProject(chatId), projectBodySchema, { method: "POST" }))
    .data;
}

/**
 * `POST /chats/{id}/save-instructions`: appends the chat's instructions to its project's and
 * clears them on the chat.
 */
export async function saveInstructions(chatId: string): Promise<ProjectResponse> {
  return (await apiFetch(API.chatSaveInstructions(chatId), projectBodySchema, { method: "POST" }))
    .data;
}

/** `GET /chats/{id}/messages`, oldest first. */
export async function fetchMessages(
  chatId: string,
  signal?: AbortSignal
): Promise<MessageResponse[]> {
  return (await apiFetch(API.chatMessages(chatId), messageListBodySchema, { signal })).items;
}

/**
 * `POST /chats/{id}/messages`: sends the message and calls `onEvent` for every event of the turn.
 * Resolves when the stream ends. Aborting `signal` only stops reading: the server keeps answering
 * and saves the reply.
 */
export async function sendMessage(
  chatId: string,
  body: SendMessageRequest,
  onEvent: (event: ChatStreamEvent) => void,
  signal?: AbortSignal
): Promise<void> {
  await apiStream(API.chatMessages(chatId), chatStreamEventSchema, onEvent, {
    method: "POST",
    body,
    signal,
  });
}
