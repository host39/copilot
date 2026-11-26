// The answer that is streaming in, built from the turn's events. Once the turn is `done`, the
// chat page reloads the saved messages and drops this state.

import type { ChatStreamEvent, MessageStatus } from "@copilot/contracts";

export type Reply = {
  /** The assistant message being written; `null` before `message.started`. */
  messageId: string | null;
  text: string;
  status: MessageStatus;
  /** The run's failure, human-readable. */
  error: string | null;
  /** The last event (`done`) arrived. */
  done: boolean;
};

export const emptyReply: Reply = {
  messageId: null,
  text: "",
  status: "streaming",
  error: null,
  done: false,
};

/** The reply after `event`. Events of other messages (a tool round's) leave the text alone. */
export function applyEvent(reply: Reply, event: ChatStreamEvent): Reply {
  switch (event.type) {
    case "message.started":
      if (event.role !== "assistant") return reply;
      return { ...reply, messageId: event.messageId, text: "", status: "streaming" };
    case "text.delta":
      if (event.messageId !== reply.messageId) return reply;
      return { ...reply, text: reply.text + event.text };
    case "message.finished":
      if (event.messageId !== reply.messageId) return reply;
      return {
        ...reply,
        status: event.status,
        // A refusal's explanation replaces the partial answer.
        text: event.refusal ? event.refusal.explanation : reply.text,
      };
    case "error":
      return { ...reply, error: event.message };
    case "done":
      return { ...reply, done: true };
    default:
      return reply;
  }
}
