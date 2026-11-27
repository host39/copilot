// The conversation's rules, apart from rendering: which answer the actions belong to, what its end
// offers (Continue, Retry), what an edit deletes, and when an answer's decisions are complete.

import type { CallDecision, MessagePartDto } from "@copilot/contracts";
import type { ToolCall, ViewMessage } from "@web/routes/chat/stream";

/** The text of a message's text parts. */
export function messageText(parts: MessagePartDto[]): string {
  return parts.flatMap((part) => (part.type === "text" ? [part.text] : [])).join("\n\n");
}

/** The last answer of the chat (the one Regenerate, Retry, and Continue act on), if any. */
export function lastAnswer(messages: ViewMessage[]): ViewMessage | undefined {
  return [...messages].reverse().find((message) => message.role === "assistant");
}

/**
 * Why an answer stopped short and offers Continue (it sends "Continue."): the output limit or
 * the tool round limit. `null` for any other ending.
 */
export function continueReason(message: ViewMessage): string | null {
  if (message.status !== "complete") return null;
  if (message.finishReason === "length") return "The answer reached the model's output limit.";
  if (message.finishReason === "tool_limit") return "The answer reached the tool round limit.";
  return null;
}

/** What `Continue` sends. */
export const CONTINUE_TEXT = "Continue.";

/** How many shown messages an edit of message `id` deletes: every later question and answer. */
export function laterCount(messages: ViewMessage[], id: string): number {
  const index = messages.findIndex((message) => message.id === id);
  if (index === -1) return 0;
  return messages.slice(index + 1).filter((message) => message.role !== "tool").length;
}

/** The confirmation an edit asks for, or `null` when it deletes nothing. */
export function editWarning(count: number): string | null {
  if (count === 0) return null;
  return count === 1
    ? "1 later message will be deleted."
    : `${count} later messages will be deleted.`;
}

/** Whether an error is a rejected key, which links to the provider settings. */
export function keyRejected(error: string): boolean {
  return error.startsWith("Key rejected");
}

/**
 * The decisions to send once every pending call has one, in call order; `null` while some call
 * still waits. Decisions for calls that are not pending are left out.
 */
export function readyDecisions(
  calls: ToolCall[],
  choices: Record<string, CallDecision>
): CallDecision[] | null {
  const list: CallDecision[] = [];
  for (const call of calls) {
    const choice = choices[call.callId];
    if (!choice) return null;
    list.push(choice);
  }
  return list;
}

/**
 * A decision for `call`: approve once or for this chat, or deny with an optional reason (passed
 * to the model). Built-in tools always ask, so they are never approved for the chat.
 */
export function choose(
  call: ToolCall,
  decision: "once" | "chat" | "deny",
  reason = ""
): CallDecision {
  if (decision === "deny") {
    const text = reason.trim();
    return { callId: call.callId, decision: "deny", ...(text ? { reason: text } : {}) };
  }
  const scope = decision === "chat" && call.serverId !== null ? "chat" : "once";
  return { callId: call.callId, decision: "approve", scope };
}
