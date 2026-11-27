// The streaming chat view's rules and cards: the last answer's actions (Retry, Continue,
// Regenerate, Copy), edits and what they delete, approval decisions, tool cards, the turn's
// helpers, and the conversation as rendered.

import type { ChatStreamEvent, MessageResponse } from "@copilot/contracts";
import { describe, expect, test } from "@rstest/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ApiError, NETWORK_ERROR } from "@web/api/client";
import { backoff } from "@web/api/retry";
import { type ListActions, MessageList } from "@web/components/Chat/MessageList";
import {
  CONTINUE_TEXT,
  choose,
  continueReason,
  editWarning,
  keyRejected,
  lastAnswer,
  laterCount,
  readyDecisions,
} from "@web/components/Chat/messages";
import { ApprovalPanel, ToolCard, toolLabel } from "@web/components/Chat/ToolCard";
import { conversationQuery, messageFiles, phaseNote, writingIds } from "@web/routes/chat/page";
import {
  apply,
  DRAFT_ID,
  emptyRun,
  receive,
  type ToolCall,
  type ViewMessage,
} from "@web/routes/chat/stream";
import { draftMessage, dropped, regenerateCut, retryCut } from "@web/routes/chat/turn";
import type { ReactNode } from "react";
import { renderToString } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";

const CHAT = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708192";
const SERVER = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708300";
const CALL = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708400";
const CALL_TWO = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708401";
const FILE = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708210";
const AT = "2026-11-26T10:00:00Z";
const ids = {
  ask: "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708500",
  answer: "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708501",
  tools: "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708502",
  again: "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708503",
  last: "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708504",
};

function message(
  id: string,
  role: MessageResponse["role"],
  extra: Partial<MessageResponse> = {}
): MessageResponse {
  return {
    id,
    chatId: CHAT,
    role,
    status: "complete",
    finishReason: role === "assistant" ? "stop" : null,
    modelId: null,
    parts: [{ type: "text", text: `${role} ${id.slice(-3)}` }],
    usage: null,
    error: null,
    createdAt: AT,
    updatedAt: AT,
    ...extra,
  };
}

const call: ToolCall = {
  type: "toolCall",
  callId: CALL,
  serverId: SERVER,
  toolName: "delete_file",
  arguments: { path: "notes.txt" },
  complete: true,
  approval: "pending",
  denyReason: null,
};

const builtin: ToolCall = {
  ...call,
  callId: CALL_TWO,
  serverId: null,
  toolName: "schedule_prompt",
};

const actions: ListActions = {
  edit: async () => true,
  regenerate: () => undefined,
  retry: () => undefined,
  continue: () => undefined,
  decide: async () => true,
};

function render(node: ReactNode): string {
  return renderToString(
    <QueryClientProvider client={new QueryClient()}>
      <MemoryRouter>{node}</MemoryRouter>
    </QueryClientProvider>
  ).replaceAll("<!-- -->", "");
}

describe("the last answer", () => {
  const history = [
    message(ids.ask, "user"),
    message(ids.answer, "assistant", { finishReason: "tool_calls" }),
    message(ids.tools, "tool"),
    message(ids.again, "assistant"),
  ];

  test("is the last assistant message, after any tool message", () => {
    expect(lastAnswer(history)?.id).toBe(ids.again);
    expect(lastAnswer([...history, message(ids.last, "tool")])?.id).toBe(ids.again);
    expect(lastAnswer([message(ids.ask, "user")])).toBeUndefined();
  });

  test("offers Continue only after the output or tool round limit", () => {
    expect(continueReason(message(ids.last, "assistant", { finishReason: "length" }))).toBe(
      "The answer reached the model's output limit."
    );
    expect(continueReason(message(ids.last, "assistant", { finishReason: "tool_limit" }))).toBe(
      "The answer reached the tool round limit."
    );
    expect(continueReason(message(ids.last, "assistant"))).toBeNull();
    expect(
      continueReason(
        message(ids.last, "assistant", { finishReason: "length", status: "cancelled" })
      )
    ).toBeNull();
    expect(CONTINUE_TEXT).toBe("Continue.");
  });

  test("regenerate replaces what follows the last question; retry the failed answer", () => {
    expect(regenerateCut(history)).toBe(ids.answer);
    expect(regenerateCut([message(ids.ask, "user")])).toBeNull();
    expect(regenerateCut([])).toBeNull();
    expect(retryCut(history)).toBeNull();
    const failed = [...history, message(ids.last, "assistant", { status: "error" })];
    expect(retryCut(failed)).toBe(ids.last);
    expect(retryCut([...failed, message(CALL, "tool")])).toBe(ids.last);
  });

  test("a rejected key links to the provider settings", () => {
    expect(keyRejected("Key rejected: open provider settings.")).toBe(true);
    expect(keyRejected("The provider is overloaded. Try again in a moment.")).toBe(false);
  });
});

describe("edits", () => {
  const history: ViewMessage[] = [
    message(ids.ask, "user"),
    message(ids.answer, "assistant"),
    message(ids.tools, "tool"),
    message(ids.again, "user"),
    message(ids.last, "assistant"),
  ];

  test("an edit deletes every later question and answer, not counting tool messages", () => {
    expect(laterCount(history, ids.ask)).toBe(3);
    expect(laterCount(history, ids.again)).toBe(1);
    expect(laterCount(history, ids.last)).toBe(0);
    expect(laterCount(history, "missing")).toBe(0);
  });

  test("asks for a confirmation only when something is deleted", () => {
    expect(editWarning(0)).toBeNull();
    expect(editWarning(1)).toBe("1 later message will be deleted.");
    expect(editWarning(3)).toBe("3 later messages will be deleted.");
  });

  test("an edit sends the message's files again", () => {
    expect(
      messageFiles(
        message(ids.ask, "user", {
          parts: [
            { type: "text", text: "Summarize" },
            { type: "file", fileId: FILE, name: "a.pdf", mediaType: "application/pdf" },
          ],
        })
      )
    ).toEqual([{ fileId: FILE, name: "a.pdf", mediaType: "application/pdf" }]);
  });

  test("the user's new message shows with its text and files until it is saved", () => {
    expect(draftMessage("Hi", [{ fileId: FILE, name: "a.png", mediaType: "image/png" }])).toEqual({
      id: DRAFT_ID,
      role: "user",
      status: "complete",
      finishReason: null,
      usage: null,
      error: null,
      parts: [
        { type: "text", text: "Hi" },
        { type: "file", fileId: FILE, name: "a.png", mediaType: "image/png" },
      ],
    });
    expect(draftMessage("", []).parts).toEqual([]);
  });
});

describe("decisions", () => {
  test("approve once, for this chat, or deny with a trimmed reason", () => {
    expect(choose(call, "once")).toEqual({ callId: CALL, decision: "approve", scope: "once" });
    expect(choose(call, "chat")).toEqual({ callId: CALL, decision: "approve", scope: "chat" });
    expect(choose(call, "deny", "  Not now. ")).toEqual({
      callId: CALL,
      decision: "deny",
      reason: "Not now.",
    });
    expect(choose(call, "deny", "   ")).toEqual({ callId: CALL, decision: "deny" });
  });

  test("built-in tools always ask, so they are never approved for the chat", () => {
    expect(choose(builtin, "chat")).toEqual({
      callId: CALL_TWO,
      decision: "approve",
      scope: "once",
    });
  });

  test("decisions go together once every pending call has one, in call order", () => {
    const calls = [call, builtin];
    const second = choose(builtin, "deny");
    const first = choose(call, "once");
    expect(readyDecisions(calls, { [CALL_TWO]: second })).toBeNull();
    expect(readyDecisions(calls, { [CALL_TWO]: second, [CALL]: first })).toEqual([first, second]);
    expect(readyDecisions([], {})).toEqual([]);
  });
});

describe("the turn", () => {
  test("reconnects after 1, 2, 4, and 8 seconds, then every 15", () => {
    expect([0, 1, 2, 3, 4, 9].map(backoff)).toEqual([1_000, 2_000, 4_000, 8_000, 15_000, 15_000]);
    expect(backoff(-1)).toBe(1_000);
  });

  test("a dropped connection is a network failure, not a refusal or the page leaving", () => {
    expect(dropped(new TypeError("network error"))).toBe(true);
    expect(dropped(new ApiError(0, { error: "Unreachable", code: NETWORK_ERROR }))).toBe(true);
    expect(dropped(new ApiError(422, { error: "Busy", code: "CHAT_BUSY" }))).toBe(false);
    expect(dropped(new DOMException("aborted", "AbortError"))).toBe(false);
  });

  test("the first event of a stream starts fresh or resumes; later ones continue", () => {
    const started: ChatStreamEvent = {
      type: "message.started",
      seq: 1,
      messageId: ids.answer,
      role: "assistant",
      windowStartId: null,
    };
    const held = apply(emptyRun, started);
    const delta: ChatStreamEvent = {
      type: "text.delta",
      seq: 2,
      messageId: ids.answer,
      index: 0,
      text: "Hi",
    };
    // A turn's stream starts a new run whatever the page held.
    expect(
      receive(held, { ...started, seq: 1, messageId: ids.last }, null, true)?.messages
    ).toHaveLength(1);
    expect(receive(held, delta, 1, true)?.seq).toBe(2);
    expect(receive(held, delta, null, false)?.seq).toBe(2);
    expect(receive(held, { ...delta, messageId: ids.last }, 1, true)).toBeNull();
    expect(receive(null, delta, null, false)?.messages[0]?.id).toBe(ids.answer);
  });

  test("only messages still streaming are being written, and only while the run goes on", () => {
    const run = apply(emptyRun, {
      type: "message.started",
      seq: 1,
      messageId: ids.answer,
      role: "assistant",
      windowStartId: null,
    });
    expect([...writingIds(run, true)]).toEqual([ids.answer]);
    expect(writingIds(run, false).size).toBe(0);
    expect(writingIds(null, true).size).toBe(0);
    expect(phaseNote("reconnecting")).toContain("reconnecting");
    expect(phaseNote("idle")).toBeNull();
    expect(phaseNote("streaming")).toBeNull();
  });

  test("the conversation waits for its messages, or shows why they failed, unless a turn streams", () => {
    const refetch = () => undefined;
    const pending = { isPending: true, isError: false, error: null, refetch };
    expect(conversationQuery(pending, false)).toMatchObject({ isPending: true, isError: false });
    expect(conversationQuery(pending, true)).toMatchObject({ isPending: false, isError: false });
    const problem = new ApiError(500, { error: "Down", code: "INTERNAL_ERROR" });
    const failed = { isPending: false, isError: true, error: problem, refetch };
    expect(conversationQuery(failed, false)).toMatchObject({ isError: true, error: problem });
    expect(conversationQuery(failed, true).isError).toBe(false);
    // A refresh that failed after the messages were read keeps showing them.
    expect(conversationQuery({ ...failed, data: [] }, false).isError).toBe(false);
  });
});

describe("tool cards", () => {
  const result = {
    type: "toolResult" as const,
    callId: CALL,
    status: "ok" as const,
    isError: false,
    content: [{ type: "text" as const, text: "Deleted." }],
    truncated: false,
  };

  test("name the tool by its title and its server, or as built-in", () => {
    const servers = new Map([[SERVER, "Files"]]);
    expect(toolLabel(call, { titles: { [CALL]: "Delete a file" }, servers })).toEqual({
      title: "Delete a file",
      source: "Files",
    });
    expect(toolLabel(call, {})).toEqual({ title: "delete_file", source: "Tool server" });
    expect(toolLabel(builtin, {})).toEqual({ title: "schedule_prompt", source: "Built-in" });
  });

  test("show arguments, status, and the result; a call without one runs while the run goes on", () => {
    const done = render(
      <ToolCard call={{ ...call, approval: "auto" }} result={result} servers={new Map()} />
    );
    expect(done).toContain("delete_file");
    expect(done).toContain("Done");
    expect(done).toContain("&quot;path&quot;: &quot;notes.txt&quot;");
    expect(done).toContain("Deleted.");
    const running = render(
      <ToolCard call={{ ...call, approval: "auto" }} result={undefined} running />
    );
    expect(running).toContain("Running");
  });

  test("approval cards offer Approve, Approve for this chat, and Deny with a reason", () => {
    const html = render(
      <ApprovalPanel calls={[call, builtin]} onDecide={async () => true} titles={{}} />
    );
    expect(html).toContain('aria-label="Tool approvals"');
    expect(html.match(/>Approve</g)).toHaveLength(2);
    // Built-in tools always ask, so only the server's call offers the chat-wide approval.
    expect(html.match(/Approve for this chat/g)).toHaveLength(1);
    expect(html.match(/>Deny</g)).toHaveLength(2);
    expect(html).toContain("Reason for denying (optional)");
    expect(html).toContain("0 of 2 decided");
  });
});

describe("the conversation", () => {
  test("the last answer offers Copy and Regenerate, and user messages Edit", () => {
    const html = render(
      <MessageList
        messages={[message(ids.ask, "user"), message(ids.answer, "assistant")]}
        actions={actions}
      />
    );
    expect(html).toContain('aria-label="Copy"');
    expect(html).toContain('aria-label="Regenerate"');
    expect(html).toContain('aria-label="Edit"');
  });

  test("while a turn runs there are no actions, and the answer being written says so", () => {
    const html = render(
      <MessageList
        messages={[
          message(ids.ask, "user"),
          message(ids.answer, "assistant", { status: "streaming", parts: [] }),
        ]}
        writing={new Set([ids.answer])}
        running
      />
    );
    expect(html).not.toContain('aria-label="Regenerate"');
    expect(html).not.toContain('aria-label="Edit"');
    expect(html).toContain("Answering...");
    expect(html).toContain('aria-busy="true"');
  });

  test("a failed answer shows its error with Retry and a link for a rejected key", () => {
    const html = render(
      <MessageList
        messages={[
          message(ids.ask, "user"),
          message(ids.answer, "assistant", {
            status: "error",
            parts: [],
            error: "Key rejected: open provider settings.",
          }),
        ]}
        actions={actions}
      />
    );
    expect(html).toContain('role="alert"');
    expect(html).toContain("Key rejected: open provider settings.");
    expect(html).toContain("Retry");
    expect(html).toContain('href="/settings/providers"');
  });

  test("only the last failed answer offers Retry", () => {
    const html = render(
      <MessageList
        messages={[
          message(ids.ask, "user"),
          message(ids.answer, "assistant", { status: "error", parts: [], error: "Failed." }),
          message(ids.again, "user"),
          message(ids.last, "assistant"),
        ]}
        actions={actions}
      />
    );
    expect(html).toContain("Failed.");
    expect(html).not.toContain("Retry");
  });

  test("an answer cut at a limit offers Continue", () => {
    const html = render(
      <MessageList
        messages={[
          message(ids.ask, "user"),
          message(ids.answer, "assistant", { finishReason: "tool_limit" }),
        ]}
        actions={actions}
      />
    );
    expect(html).toContain("The answer reached the tool round limit.");
    expect(html).toContain(">Continue<");
  });

  test("an answer waiting for decisions shows the approval cards; a busy chat only says it waits", () => {
    const waiting = message(ids.answer, "assistant", {
      status: "awaiting_approval",
      finishReason: "tool_calls",
      parts: [call],
    });
    const open = render(
      <MessageList messages={[message(ids.ask, "user"), waiting]} actions={actions} />
    );
    expect(open).toContain('aria-label="Tool approvals"');
    expect(open).not.toContain('aria-label="Regenerate"');
    const busy = render(<MessageList messages={[message(ids.ask, "user"), waiting]} running />);
    expect(busy).not.toContain('aria-label="Tool approvals"');
    expect(busy).toContain("Waiting for a tool decision.");
  });

  test("a refusal shows its explanation; a stop says Stopped", () => {
    const html = render(
      <MessageList
        messages={[
          message(ids.ask, "user"),
          message(ids.answer, "assistant", {
            status: "refused",
            finishReason: "refusal",
            error: "I cannot help with that.",
          }),
          message(ids.again, "user"),
          message(ids.last, "assistant", { status: "cancelled" }),
        ]}
      />
    );
    expect(html).toContain("I cannot help with that.");
    expect(html).toContain("Stopped.");
  });

  test("messages before the context window are marked as not sent", () => {
    const html = render(
      <MessageList
        messages={[
          message(ids.ask, "user"),
          message(ids.answer, "assistant"),
          message(ids.again, "user"),
        ]}
        windowStartId={ids.again}
      />
    );
    expect(html).toContain("Earlier messages not sent");
    const first = render(
      <MessageList messages={[message(ids.ask, "user")]} windowStartId={ids.ask} />
    );
    expect(first).not.toContain("Earlier messages not sent");
  });

  test("the user's unsaved message has no anchor and cannot be edited yet", () => {
    const html = render(
      <MessageList messages={[draftMessage("Draft text", [])]} actions={actions} />
    );
    expect(html).toContain("Draft text");
    expect(html).not.toContain('id="message-draft"');
    expect(html).not.toContain('aria-label="Edit"');
  });
});
