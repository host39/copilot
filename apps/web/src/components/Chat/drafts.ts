// What the user was writing, kept while they look elsewhere: the composer's text and attached
// files per chat (or per new chat draft, by project), and a new chat's model, reasoning, tools,
// and Speak replies. Drafts live in memory for this page only (a reload starts empty, and nothing
// reaches the disk), and every one is forgotten when the session changes, so the next account
// never sees them.

import type { ReasoningLevel } from "@copilot/contracts";
import { subscribeToken } from "@web/auth/token";
import type { Attachment } from "@web/components/Chat/composer";
import type { ToolChoice } from "@web/components/Chat/ToolsMenu";

/** The composer's draft: its text, its ready files (with their bytes), and its next file key. */
export type ComposerDraft = {
  text: string;
  items: Attachment[];
  files: Map<string, File>;
  counter: number;
};

/** A new chat's choices besides the message. */
export type StartDraft = {
  choice?: { modelId: string; reasoning: ReasoningLevel | null };
  tools: ToolChoice;
  speak: boolean;
};

/** How many drafts of each kind are kept; the oldest goes first. */
export const DRAFTS_MAX = 20;

const composers = new Map<string, ComposerDraft>();
const starts = new Map<string, StartDraft>();

subscribeToken(() => {
  composers.clear();
  starts.clear();
});

/** The composer's draft key: a chat, or the new chat screen of a project (or of none). */
export function draftKey(scope: { chatId: string } | { projectId: string | null }): string {
  return "chatId" in scope ? `chat:${scope.chatId}` : `new:${scope.projectId ?? ""}`;
}

function keep<T>(store: Map<string, T>, key: string, value: T): void {
  store.delete(key);
  store.set(key, value);
  while (store.size > DRAFTS_MAX) {
    const oldest = store.keys().next().value;
    if (oldest === undefined) break;
    store.delete(oldest);
  }
}

/** The composer draft under `key`, if one is kept. */
export function readComposer(key: string): ComposerDraft | undefined {
  return composers.get(key);
}

/**
 * Keeps what the composer holds under `key`: only files that finished uploading (or wait for the
 * first send) count, and an empty draft is dropped.
 */
export function keepComposer(key: string, draft: ComposerDraft): void {
  const items = draft.items.filter((item) => item.status === "ready");
  if (draft.text === "" && items.length === 0) {
    composers.delete(key);
    return;
  }
  const files = new Map(
    items.flatMap((item) => {
      const file = draft.files.get(item.key);
      return file ? [[item.key, file] as const] : [];
    })
  );
  keep(composers, key, { ...draft, items, files });
}

/** The new chat choices under `key`, if kept. */
export function readStart(key: string): StartDraft | undefined {
  return starts.get(key);
}

export function keepStart(key: string, draft: StartDraft): void {
  keep(starts, key, draft);
}

export function dropStart(key: string): void {
  starts.delete(key);
}
