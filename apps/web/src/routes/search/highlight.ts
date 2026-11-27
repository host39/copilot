// Search results: the words of a query, the marked parts of a snippet, and links to the match.
// The server matches each word as a word prefix (`plainto_tsquery` with `:*`), so the snippet
// marks the parts of words that start with a query word.

import type { SearchHitResponse } from "@copilot/contracts";
import { link } from "@web/routes/path";

/** Core's longest search text, in characters. */
export const SEARCH_MAX_CHARS = 200;

/** Why `query` cannot be searched for, or `null` when it can. */
export function queryProblem(query: string): string | null {
  const text = query.trim();
  if (text === "") return "Enter text to search for.";
  if ([...text].length > SEARCH_MAX_CHARS) {
    return `Search text must be at most ${SEARCH_MAX_CHARS} characters.`;
  }
  return null;
}

/** The distinct words of `query`, lowercased, longest first (so a longer word wins a mark). */
export function terms(query: string): string[] {
  const words = query.toLowerCase().match(/[\p{L}\p{N}]+/gu) ?? [];
  return [...new Set(words)].sort((a, b) => b.length - a.length);
}

export type Segment = { text: string; match: boolean };

function literal(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** `snippet` cut into plain and matched parts; a match starts a word and ignores case. */
export function highlight(snippet: string, words: string[]): Segment[] {
  if (words.length === 0) return snippet ? [{ text: snippet, match: false }] : [];
  const pattern = new RegExp(`(?<![\\p{L}\\p{N}])(?:${words.map(literal).join("|")})`, "giu");
  const segments: Segment[] = [];
  let last = 0;
  for (const found of snippet.matchAll(pattern)) {
    const start = found.index ?? 0;
    if (start > last) segments.push({ text: snippet.slice(last, start), match: false });
    segments.push({ text: found[0], match: true });
    last = start + found[0].length;
  }
  if (last < snippet.length) segments.push({ text: snippet.slice(last), match: false });
  return segments;
}

/** The id of a message's element in the chat view, which a search result scrolls to. */
export function messageAnchor(messageId: string): string {
  return `message-${messageId}`;
}

/** Where a result goes: the chat, at the matching message when there is one. */
export function hitLink(hit: SearchHitResponse): string {
  const chat = link.chat(hit.chatId);
  return hit.messageId ? `${chat}#${messageAnchor(hit.messageId)}` : chat;
}
