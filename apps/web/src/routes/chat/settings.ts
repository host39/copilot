// The chat screen's settings, apart from streaming: rename, the model notice, whether tools are on
// (which turns reasoning off on some connections), the overflow menu's actions, and the
// instructions drawer.

import type {
  ChatResponse,
  MessageResponse,
  ModelResponse,
  ProjectResponse,
} from "@copilot/contracts";
import { z } from "zod";

/** Core's `NAME_MAX_CHARS`, which also bounds a chat's title. */
const TITLE_MAX_CHARS = 100;

/** A new title, by core's rules (`Chat::with_title`). */
export const titleSchema = z
  .string()
  .trim()
  .min(1, "Title is required.")
  .refine((value) => !/\p{Cc}/u.test(value), "Title must be a single line.")
  .refine(
    (value) => [...value].length <= TITLE_MAX_CHARS,
    `Title must be at most ${TITLE_MAX_CHARS} characters.`
  );

/** What the composer says when the chat cannot answer with its model, or `null`. */
export function modelNotice(
  chat: Pick<ChatResponse, "modelId">,
  models: ModelResponse[] | undefined
): string | null {
  if (chat.modelId === null) return "Choose a model to continue.";
  if (models && !models.some((model) => model.id === chat.modelId)) {
    return "This chat's model is turned off or no longer listed. Choose another one to continue.";
  }
  return null;
}

/**
 * Whether the chat's next turn offers tools: the project's servers with the chat's overrides
 * applied, or any built-in tools (the chat's, else the project's).
 */
export function toolsOn(
  chat: Pick<ChatResponse, "toolOverrides" | "builtinTools">,
  project: Pick<ProjectResponse, "toolServerIds" | "builtinTools"> | null | undefined
): boolean {
  const servers = new Set(project?.toolServerIds ?? []);
  for (const item of chat.toolOverrides) {
    if (item.enabled) servers.add(item.serverId);
    else servers.delete(item.serverId);
  }
  const builtin = chat.builtinTools ?? project?.builtinTools ?? [];
  return servers.size > 0 || builtin.length > 0;
}

export type MenuAction =
  | "project"
  | "instructions"
  | "move"
  | "pin"
  | "schedule"
  | "save-as-project"
  | "save-instructions"
  | "delete";

/**
 * The overflow menu's actions: a project chat offers its project and Save instructions to
 * project (while it has instructions); a standalone chat offers Save as project. Every chat offers
 * Schedule this prompt.
 */
export function menuActions(chat: Pick<ChatResponse, "projectId" | "instructions">): MenuAction[] {
  const inProject = chat.projectId !== null;
  return [
    ...(inProject ? (["project"] as const) : []),
    "instructions",
    "move",
    "pin",
    "schedule",
    ...(inProject
      ? chat.instructions.trim()
        ? (["save-instructions"] as const)
        : []
      : (["save-as-project"] as const)),
    "delete",
  ];
}

/** `addition` after `current`, separated by a blank line ("Insert from project..."). */
export function insertInstructions(current: string, addition: string): string {
  const before = current.trimEnd();
  const text = addition.trim();
  if (!text) return current;
  return before ? `${before}\n\n${text}` : text;
}

/** The tool results of a chat by call id, for the history's tool calls. */
export function toolResults(messages: Pick<MessageResponse, "parts">[]) {
  const results = new Map<
    string,
    Extract<MessageResponse["parts"][number], { type: "toolResult" }>
  >();
  for (const message of messages) {
    for (const part of message.parts) {
      if (part.type === "toolResult") results.set(part.callId, part);
    }
  }
  return results;
}

/**
 * The context meter: the last answer's input tokens against the model's context window, or `null`
 * while either is unknown.
 */
export function contextUse(
  messages: Pick<MessageResponse, "role" | "usage">[],
  model: Pick<ModelResponse, "contextWindow"> | undefined
): { used: number; window: number; percent: number } | null {
  const window = model?.contextWindow ?? null;
  const last = [...messages]
    .reverse()
    .find((message) => message.role === "assistant" && message.usage?.inputTokens != null);
  const used = last?.usage?.inputTokens ?? null;
  if (window === null || window <= 0 || used === null) return null;
  return { used, window, percent: Math.round((used / window) * 100) };
}
