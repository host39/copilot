// A run's messages, built from its stream events (`ChatStreamEvent`) as they arrive, and how they
// join the saved messages on screen.
//
// Every event carries the run's `seq`. `apply` takes only events past the last `seq` it applied, so
// a replay over a run it already holds changes nothing. A stream reattached after a dropped
// connection asks for the events after that `seq`; `resume` decides where its first event goes:
// the run it continues, a fresh run when the server replays from the start (a replay merges
// deltas, so it is rebuilt rather than added to text already shown), or nowhere when the stream
// belongs to another run (attach again from the start).

import type {
  CallDecision,
  ChatStreamEvent,
  MessagePartDto,
  MessageResponse,
  Role,
} from "@copilot/contracts";

/** A message on screen: saved, or being written by a run. */
export type ViewMessage = Pick<
  MessageResponse,
  "id" | "role" | "status" | "finishReason" | "parts" | "usage" | "error"
>;

export type ToolCall = Extract<MessagePartDto, { type: "toolCall" }>;
export type ToolResult = Extract<MessagePartDto, { type: "toolResult" }>;
type TextPart = Extract<MessagePartDto, { type: "text" | "reasoning" }>;

/** A text or reasoning part at its place in the message. */
type Slot = { index: number; part: TextPart };

/** A message the run is writing. */
export type RunMessage = Omit<ViewMessage, "parts"> & {
  /** Text and reasoning parts, by their place in the message. */
  slots: Slot[];
  /** An answer's tool calls, or a tool message's results, in the order they came. */
  tools: (ToolCall | ToolResult)[];
};

export type RunError = { message: string; code: string };

export type Run = {
  /** The last `seq` applied; `0` before any event. */
  seq: number;
  /** In the order they started. */
  messages: RunMessage[];
  /** The display titles of the run's tool calls, by call id. */
  titles: Record<string, string>;
  /** The first message the model saw; `null` when it saw the whole chat. */
  windowStartId: string | null;
  /** The run's failure, human-readable. */
  error: RunError | null;
  /** `done` arrived: the run is over and its messages are saved. */
  done: boolean;
};

export const emptyRun: Run = {
  seq: 0,
  messages: [],
  titles: {},
  windowStartId: null,
  error: null,
  done: false,
};

function blank(id: string, role: Role): RunMessage {
  return {
    id,
    role,
    status: "streaming",
    finishReason: null,
    usage: null,
    error: null,
    slots: [],
    tools: [],
  };
}

/**
 * `run` with `change` applied to message `id`. An event of a message that never started (its
 * `message.started` was lost) starts it with `role`.
 */
function update(
  run: Run,
  id: string,
  role: Role,
  change: (message: RunMessage) => RunMessage
): Run {
  const found = run.messages.some((message) => message.id === id);
  const messages = found ? run.messages : [...run.messages, blank(id, role)];
  return {
    ...run,
    messages: messages.map((message) => (message.id === id ? change(message) : message)),
  };
}

/** The message with a part of `kind` at `index` (kept when it is there already). */
function slot(message: RunMessage, index: number, kind: TextPart["type"]): RunMessage {
  if (message.slots.some((item) => item.index === index)) return message;
  const slots = [...message.slots, { index, part: { type: kind, text: "" } }];
  return { ...message, slots: slots.sort((a, b) => a.index - b.index) };
}

function append(
  message: RunMessage,
  index: number,
  kind: TextPart["type"],
  text: string
): RunMessage {
  const placed = slot(message, index, kind);
  return {
    ...placed,
    slots: placed.slots.map((item) =>
      item.index === index ? { index, part: { ...item.part, text: item.part.text + text } } : item
    ),
  };
}

/** The message with `part` in place of the call or result it repeats, else added. */
function tool(message: RunMessage, part: ToolCall | ToolResult): RunMessage {
  const same = (item: ToolCall | ToolResult) =>
    item.type === part.type && item.callId === part.callId;
  const tools = message.tools.some(same)
    ? message.tools.map((item) => (same(item) ? part : item))
    : [...message.tools, part];
  return { ...message, tools };
}

/** The run after `event`. Events at or below the last applied `seq`, or after `done`, change nothing. */
export function apply(run: Run, event: ChatStreamEvent): Run {
  if (event.seq <= run.seq || run.done) return run;
  const next: Run = { ...run, seq: event.seq };
  switch (event.type) {
    case "message.started":
      return {
        ...update(next, event.messageId, event.role, (message) => ({
          ...message,
          role: event.role,
        })),
        windowStartId: event.windowStartId,
      };
    case "part.started": {
      // Tool calls arrive whole with `tool.requested`; spoken audio plays with voice (P4.1).
      const kind = event.kind;
      return update(next, event.messageId, "assistant", (message) =>
        kind === "text" || kind === "reasoning" ? slot(message, event.index, kind) : message
      );
    }
    case "text.delta":
      return update(next, event.messageId, "assistant", (message) =>
        append(message, event.index, "text", event.text)
      );
    case "reasoning.delta":
      return update(next, event.messageId, "assistant", (message) =>
        append(message, event.index, "reasoning", event.text)
      );
    case "audio.delta":
      return next;
    case "tool.requested":
      return {
        ...update(next, event.messageId, "assistant", (message) =>
          tool(message, {
            type: "toolCall",
            callId: event.callId,
            serverId: event.serverId,
            toolName: event.toolName,
            arguments: event.arguments,
            complete: true,
            approval: event.needsApproval ? "pending" : "auto",
            denyReason: null,
          })
        ),
        titles: { ...next.titles, [event.callId]: event.title },
      };
    case "tool.finished":
      return update(next, event.messageId, "tool", (message) =>
        tool(message, {
          type: "toolResult",
          callId: event.callId,
          status: event.status,
          isError: event.status !== "ok",
          content: event.output,
          truncated: event.truncated,
        })
      );
    case "message.finished":
      return update(next, event.messageId, "assistant", (message) => ({
        ...message,
        status: event.status,
        finishReason: event.finishReason,
        usage: event.usage,
        // A refusal's explanation replaces the partial answer.
        slots: event.refusal
          ? message.slots.filter((item) => item.part.type !== "text")
          : message.slots,
        error:
          event.refusal?.explanation ??
          message.error ??
          (event.status === "error" ? (next.error?.message ?? null) : null),
      }));
    case "error": {
      const error = { message: event.message, code: event.code };
      // The answer being written failed with it.
      const writing = [...next.messages]
        .reverse()
        .find((message) => message.role === "assistant" && message.status === "streaming");
      const failed = writing
        ? update(next, writing.id, "assistant", (message) => ({
            ...message,
            error: message.error ?? error.message,
          }))
        : next;
      return { ...failed, error };
    }
    case "done":
      return { ...next, done: true };
  }
}

/**
 * Whether `event`, the first of a stream reattached after `run.seq`, continues `run`: it starts a
 * message, ends the run, or belongs to a message the run holds.
 */
export function continues(run: Run, event: ChatStreamEvent): boolean {
  switch (event.type) {
    case "message.started":
    case "error":
    case "done":
      return true;
    default:
      return run.messages.some((message) => message.id === event.messageId);
  }
}

/**
 * Where the first event of a stream reattached after `run.seq` applies: `run` when the stream
 * continues it, a fresh run when the server replays from the start, or `null` when the stream
 * belongs to another run, which only a replay from the start can rebuild.
 */
export function resume(run: Run, event: ChatStreamEvent): Run | null {
  if (event.seq <= run.seq) return emptyRun;
  return continues(run, event) ? run : null;
}

/**
 * A run message's parts in their order: text and reasoning at their places, tool calls filling
 * the places between them (an answer's calls are never announced by place), then the rest.
 */
function layout(message: RunMessage): MessagePartDto[] {
  const parts: MessagePartDto[] = [];
  const queue = [...message.tools];
  let position = 0;
  for (const item of message.slots) {
    while (position < item.index && queue.length > 0) {
      const next = queue.shift();
      if (next) parts.push(next);
      position += 1;
    }
    parts.push(item.part);
    position = Math.max(position, item.index) + 1;
  }
  return [...parts, ...queue];
}

/** The run's messages as the screen shows them. */
export function view(run: Run): ViewMessage[] {
  return run.messages.map((message) => ({
    id: message.id,
    role: message.role,
    status: message.status,
    finishReason: message.finishReason,
    parts: layout(message),
    usage: message.usage,
    error: message.error,
  }));
}

/** A turn the user just started, which the saved messages may not show yet. */
export type Draft = {
  /** The first saved message the turn replaces (edit, regenerate, retry); it and every later one are hidden. */
  cut: string | null;
  /** The user's new message, shown until the saved messages hold it. */
  message: ViewMessage | null;
  /** The last saved message kept when the turn started; a user message saved after it is the new one. */
  after: string | null;
};

/** The id the screen gives the user's message until it is saved. */
export const DRAFT_ID = "draft";

/**
 * The conversation on screen: the saved messages (without those a new turn replaces), the user's
 * new message until it is saved, then the run's messages. A saved message the run is still
 * writing (`streaming`) shows the run's copy; a saved message that is final wins over the run's.
 */
export function timeline(
  saved: MessageResponse[],
  run: Run | null,
  draft: Draft | null
): ViewMessage[] {
  const cut = draft?.cut ? saved.findIndex((message) => message.id === draft.cut) : -1;
  const kept: ViewMessage[] = cut === -1 ? saved : saved.slice(0, cut);
  const live = run ? view(run) : [];
  const writing = new Map(live.map((message) => [message.id, message]));
  const shown = new Set(kept.map((message) => message.id));
  const merged = kept.map((message) =>
    message.status === "streaming" ? (writing.get(message.id) ?? message) : message
  );

  const after = draft?.after ? kept.findIndex((message) => message.id === draft.after) : -1;
  const saves = kept.slice(after + 1).some((message) => message.role === "user");
  const own = draft?.message && !saves ? [draft.message] : [];

  return [...merged, ...own, ...live.filter((message) => !shown.has(message.id))];
}

/** The pending tool calls of `message` (an answer waiting for decisions). */
export function pendingCalls(message: ViewMessage): ToolCall[] {
  if (message.status !== "awaiting_approval") return [];
  return message.parts.flatMap((part) =>
    part.type === "toolCall" && part.approval === "pending" ? [part] : []
  );
}

/**
 * The saved messages once `decisions` are sent for `messageId`: its pending calls approved or
 * denied (with the reason) and the message no longer waiting, as the server records them.
 */
export function decided(
  messages: MessageResponse[],
  messageId: string,
  decisions: CallDecision[]
): MessageResponse[] {
  const choices = new Map(decisions.map((item) => [item.callId, item]));
  return messages.map((message) => {
    if (message.id !== messageId || message.status !== "awaiting_approval") return message;
    return {
      ...message,
      status: "complete",
      parts: message.parts.map((part) => {
        const choice = part.type === "toolCall" ? choices.get(part.callId) : undefined;
        if (part.type !== "toolCall" || part.approval !== "pending" || !choice) return part;
        return choice.decision === "approve"
          ? { ...part, approval: "approved" }
          : { ...part, approval: "denied", denyReason: choice.reason?.trim() || null };
      }),
    };
  });
}
