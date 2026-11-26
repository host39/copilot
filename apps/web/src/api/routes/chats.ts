// Chats and their turns. `sendMessage` answers with the turn's server-sent events, each a
// `ChatStreamEvent`, ending with `done`.

import {
  type ChatResponse,
  type ChatStreamEvent,
  type CreateChatRequest,
  chatBodySchema,
  chatStreamEventSchema,
  type MessageResponse,
  messageListBodySchema,
  type SendMessageRequest,
} from "@copilot/contracts";
import { apiFetch, apiStream } from "@web/api/client";
import API from "@web/api/endpoints";

/** `POST /chats`: the new chat, created with its first message. */
export async function createChat(body: CreateChatRequest): Promise<ChatResponse> {
  return (await apiFetch(API.chats, chatBodySchema, { method: "POST", body })).data;
}

/** `GET /chats/{id}`. */
export async function fetchChat(chatId: string, signal?: AbortSignal): Promise<ChatResponse> {
  return (await apiFetch(API.chat(chatId), chatBodySchema, { signal })).data;
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
