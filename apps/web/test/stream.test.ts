// The stream reducer: every event, replay from a `seq`, events out of order, how a run joins the
// saved messages, decisions, and the streaming chat routes.

import type { ChatStreamEvent, MessageResponse } from "@copilot/contracts";
import { afterEach, describe, expect, test } from "@rstest/core";
import { resetTransport, setTransport } from "@web/api/client";
import { attach, decide, regenerate, retry, stop } from "@web/api/routes/chats";
import { clearToken, setToken } from "@web/auth/token";
import {
  apply,
  continues,
  DRAFT_ID,
  decided,
  emptyRun,
  pendingCalls,
  type Run,
  resume,
  timeline,
  unreachableNote,
  view,
} from "@web/routes/chat/stream";

const CHAT = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708192";
const USER = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708110";
const ANSWER = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708111";
const TOOLS = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708112";
const NEXT = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708113";
const CALL = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708400";
const CALL_TWO = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708401";
const SERVER = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708300";
const AT = "2026-11-26T10:00:00Z";

const run = (events: ChatStreamEvent[], from: Run = emptyRun) => events.reduce(apply, from);

/** An event without its `seq`, for each variant. */
type Unnumbered = ChatStreamEvent extends infer Event
  ? Event extends unknown
    ? Omit<Event, "seq">
    : never
  : never;

const usage = {
  inputTokens: 120,
  outputTokens: 30,
  reasoningTokens: null,
  cacheReadTokens: null,
  cacheWriteTokens: null,
};

/** A turn with reasoning, text, a tool round, and a final answer, numbered from 1. */
function turn(): ChatStreamEvent[] {
  const events: Unnumbered[] = [
    { type: "message.started", messageId: ANSWER, role: "assistant", windowStartId: USER },
    { type: "part.started", messageId: ANSWER, index: 0, kind: "reasoning" },
    { type: "reasoning.delta", messageId: ANSWER, index: 0, text: "Look it " },
    { type: "reasoning.delta", messageId: ANSWER, index: 0, text: "up." },
    { type: "part.started", messageId: ANSWER, index: 1, kind: "text" },
    { type: "text.delta", messageId: ANSWER, index: 1, text: "Checking" },
    { type: "text.delta", messageId: ANSWER, index: 1, text: " the weather." },
    {
      type: "tool.requested",
      messageId: ANSWER,
      callId: CALL,
      serverId: SERVER,
      toolName: "forecast",
      title: "Weather forecast",
      arguments: { city: "Oslo" },
      needsApproval: false,
    },
    {
      type: "message.finished",
      messageId: ANSWER,
      status: "complete",
      finishReason: "tool_calls",
      refusal: null,
      usage,
    },
    { type: "message.started", messageId: TOOLS, role: "tool", windowStartId: USER },
    {
      type: "tool.finished",
      messageId: TOOLS,
      callId: CALL,
      status: "ok",
      output: [{ type: "text", text: "Sunny" }],
      truncated: false,
    },
    {
      type: "message.finished",
      messageId: TOOLS,
      status: "complete",
      finishReason: null,
      refusal: null,
      usage: null,
    },
    { type: "message.started", messageId: NEXT, role: "assistant", windowStartId: USER },
    { type: "part.started", messageId: NEXT, index: 0, kind: "text" },
    { type: "text.delta", messageId: NEXT, index: 0, text: "It is " },
    { type: "text.delta", messageId: NEXT, index: 0, text: "sunny." },
    {
      type: "message.finished",
      messageId: NEXT,
      status: "complete",
      finishReason: "stop",
      refusal: null,
      usage,
    },
    { type: "done" },
  ];
  return events.map((event, index) => ({ ...event, seq: index + 1 }) as ChatStreamEvent);
}

/** The run's replay buffer: consecutive deltas of a part merged, each with its last `seq`. */
function buffer(events: ChatStreamEvent[]): ChatStreamEvent[] {
  const merged: ChatStreamEvent[] = [];
  for (const event of events) {
    const last = merged.at(-1);
    if (
      (event.type === "text.delta" || event.type === "reasoning.delta") &&
      last?.type === event.type &&
      last.messageId === event.messageId &&
      last.index === event.index
    ) {
      merged[merged.length - 1] = { ...last, seq: event.seq, text: last.text + event.text };
    } else {
      merged.push(event);
    }
  }
  return merged;
}

function saved(
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
    parts: [{ type: "text", text: id }],
    usage: null,
    error: null,
    createdAt: AT,
    updatedAt: AT,
    ...extra,
  };
}

describe("every event", () => {
  test("a whole turn builds its messages, tool calls, and results", () => {
    const result = run(turn());
    expect(result.seq).toBe(18);
    expect(result.done).toBe(true);
    expect(result.windowStartId).toBe(USER);
    expect(result.titles).toEqual({ [CALL]: "Weather forecast" });
    expect(view(result)).toEqual([
      {
        id: ANSWER,
        role: "assistant",
        status: "complete",
        finishReason: "tool_calls",
        usage,
        error: null,
        parts: [
          { type: "reasoning", text: "Look it up." },
          { type: "text", text: "Checking the weather." },
          {
            type: "toolCall",
            callId: CALL,
            serverId: SERVER,
            toolName: "forecast",
            arguments: { city: "Oslo" },
            complete: true,
            approval: "auto",
            denyReason: null,
          },
        ],
      },
      {
        id: TOOLS,
        role: "tool",
        status: "complete",
        finishReason: null,
        usage: null,
        error: null,
        parts: [
          {
            type: "toolResult",
            callId: CALL,
            status: "ok",
            isError: false,
            content: [{ type: "text", text: "Sunny" }],
            truncated: false,
          },
        ],
      },
      {
        id: NEXT,
        role: "assistant",
        status: "complete",
        finishReason: "stop",
        usage,
        error: null,
        parts: [{ type: "text", text: "It is sunny." }],
      },
    ]);
  });

  test("a message streams until it finishes", () => {
    const partial = view(run(turn().slice(0, 6)));
    expect(partial).toHaveLength(1);
    expect(partial[0]?.status).toBe("streaming");
    expect(partial[0]?.parts).toEqual([
      { type: "reasoning", text: "Look it up." },
      { type: "text", text: "Checking" },
    ]);
  });

  test("a call that needs approval is pending, and failed results are errors", () => {
    const result = run([
      {
        type: "message.started",
        seq: 1,
        messageId: ANSWER,
        role: "assistant",
        windowStartId: null,
      },
      {
        type: "tool.requested",
        seq: 2,
        messageId: ANSWER,
        callId: CALL,
        serverId: null,
        toolName: "schedule_prompt",
        title: "schedule_prompt",
        arguments: "{not json",
        needsApproval: true,
      },
      {
        type: "message.finished",
        seq: 3,
        messageId: ANSWER,
        status: "awaiting_approval",
        finishReason: "tool_calls",
        refusal: null,
        usage: null,
      },
      {
        type: "tool.finished",
        seq: 4,
        messageId: TOOLS,
        callId: CALL_TWO,
        status: "timeout",
        output: [{ type: "text", text: "Timed out." }],
        truncated: true,
      },
    ]);
    const [answer, results] = view(result);
    expect(answer?.status).toBe("awaiting_approval");
    expect(answer?.parts[0]).toMatchObject({ approval: "pending", arguments: "{not json" });
    expect(answer && pendingCalls(answer).map((call) => call.callId)).toEqual([CALL]);
    expect(results?.role).toBe("tool");
    expect(results?.parts[0]).toMatchObject({ status: "timeout", isError: true, truncated: true });
  });

  test("tool calls fill the places between the text parts", () => {
    const result = run([
      { type: "part.started", seq: 1, messageId: ANSWER, index: 0, kind: "text" },
      { type: "text.delta", seq: 2, messageId: ANSWER, index: 0, text: "First" },
      { type: "part.started", seq: 3, messageId: ANSWER, index: 2, kind: "text" },
      { type: "text.delta", seq: 4, messageId: ANSWER, index: 2, text: "Then" },
      ...[CALL, CALL_TWO].map(
        (callId, offset) =>
          ({
            type: "tool.requested",
            seq: 5 + offset,
            messageId: ANSWER,
            callId,
            serverId: SERVER,
            toolName: "lookup",
            title: "Lookup",
            arguments: {},
            needsApproval: false,
          }) as ChatStreamEvent
      ),
    ]);
    const parts = view(result)[0]?.parts ?? [];
    expect(parts.map((part) => (part.type === "toolCall" ? part.callId : part.type))).toEqual([
      "text",
      CALL,
      "text",
      CALL_TWO,
    ]);
  });

  test("audio and tool call parts wait for their own events", () => {
    const result = run([
      {
        type: "message.started",
        seq: 1,
        messageId: ANSWER,
        role: "assistant",
        windowStartId: null,
      },
      { type: "part.started", seq: 2, messageId: ANSWER, index: 0, kind: "toolCall" },
      { type: "part.started", seq: 3, messageId: ANSWER, index: 1, kind: "audio" },
      { type: "audio.delta", seq: 4, messageId: ANSWER, data: "AAAA", format: "pcm16" },
    ]);
    expect(result.seq).toBe(4);
    expect(view(result)[0]?.parts).toEqual([]);
  });

  test("a refusal's explanation replaces the partial answer", () => {
    const result = run([
      {
        type: "message.started",
        seq: 1,
        messageId: ANSWER,
        role: "assistant",
        windowStartId: null,
      },
      { type: "text.delta", seq: 2, messageId: ANSWER, index: 0, text: "Sure, here" },
      {
        type: "message.finished",
        seq: 3,
        messageId: ANSWER,
        status: "refused",
        finishReason: "refusal",
        refusal: { category: "cyber", explanation: "I cannot help with that." },
        usage: null,
      },
    ]);
    const [answer] = view(result);
    expect(answer?.status).toBe("refused");
    expect(answer?.parts).toEqual([]);
    expect(answer?.error).toBe("I cannot help with that.");
  });

  test("an error belongs to the answer being written and to the run", () => {
    const result = run([
      {
        type: "message.started",
        seq: 1,
        messageId: ANSWER,
        role: "assistant",
        windowStartId: null,
      },
      { type: "text.delta", seq: 2, messageId: ANSWER, index: 0, text: "Half" },
      {
        type: "error",
        seq: 3,
        message: "Key rejected: open provider settings.",
        code: "PROVIDER_AUTH",
      },
      {
        type: "message.finished",
        seq: 4,
        messageId: ANSWER,
        status: "error",
        finishReason: "error",
        refusal: null,
        usage: null,
      },
      { type: "done", seq: 5 },
    ]);
    expect(result.error).toEqual({
      message: "Key rejected: open provider settings.",
      code: "PROVIDER_AUTH",
    });
    expect(view(result)[0]).toMatchObject({
      status: "error",
      error: "Key rejected: open provider settings.",
      parts: [{ type: "text", text: "Half" }],
    });
  });

  test("an error before any answer belongs to the run only", () => {
    const result = run([
      { type: "error", seq: 1, message: "Something went wrong.", code: "INTERNAL" },
      { type: "done", seq: 2 },
    ]);
    expect(result.messages).toEqual([]);
    expect(result.error?.code).toBe("INTERNAL");
    expect(result.done).toBe(true);
  });

  test("nothing changes after done", () => {
    const finished = run(turn());
    const late: ChatStreamEvent = {
      type: "text.delta",
      seq: 99,
      messageId: NEXT,
      index: 0,
      text: " Extra",
    };
    expect(apply(finished, late)).toBe(finished);
  });
});

describe("replay from a seq", () => {
  test("the replay buffer builds the same messages as the live events", () => {
    const replay = buffer(turn());
    expect(replay.length).toBeLessThan(turn().length);
    expect(replay.map((event) => event.seq)).not.toEqual(turn().map((event) => event.seq));
    expect(view(run(replay))).toEqual(view(run(turn())));
    expect(run(replay).seq).toBe(18);
  });

  test("replaying events the run already holds changes nothing", () => {
    const live = run(turn().slice(0, 10));
    expect(run(turn().slice(0, 10), live)).toBe(live);
    expect(
      run(
        buffer(turn()).filter((event) => event.seq <= 10),
        live
      )
    ).toBe(live);
  });

  test("a stream after the last seq continues the run", () => {
    const events = turn();
    const before = run(events.slice(0, 9));
    const first = events[9];
    if (!first) throw new Error("missing event");
    expect(resume(before, first)).toBe(before);
    expect(view(run(events.slice(9), before))).toEqual(view(run(events)));
  });

  test("a stream that replays from the start rebuilds a fresh run", () => {
    const events = turn();
    // The connection dropped inside a merged part: adding the replay would repeat "Checking".
    const before = run(events.slice(0, 6));
    const replay = buffer(events);
    const first = replay[0];
    if (!first) throw new Error("missing event");
    const base = resume(before, first);
    expect(base).toEqual(emptyRun);
    const rebuilt = run(replay, base ?? emptyRun);
    expect(view(rebuilt)).toEqual(view(run(events)));
  });

  test("a stream of another run cannot continue this one", () => {
    const before = run(turn().slice(0, 4));
    const other: ChatStreamEvent = {
      type: "text.delta",
      seq: 12,
      messageId: "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708999",
      index: 0,
      text: "Elsewhere",
    };
    expect(continues(before, other)).toBe(false);
    expect(resume(before, other)).toBeNull();
    const ending: ChatStreamEvent = { type: "done", seq: 12 };
    expect(continues(before, ending)).toBe(true);
    const started: ChatStreamEvent = {
      type: "message.started",
      seq: 12,
      messageId: NEXT,
      role: "assistant",
      windowStartId: null,
    };
    expect(continues(before, started)).toBe(true);
  });
});

describe("events out of order", () => {
  test("an event at or below the last seq is dropped", () => {
    const events = turn();
    const result = run([events[0], events[2], events[1], events[2]] as ChatStreamEvent[]);
    expect(result.seq).toBe(3);
    expect(view(result)[0]?.parts).toEqual([{ type: "reasoning", text: "Look it " }]);
  });

  test("a delta before its message and part starts them", () => {
    const result = run([
      { type: "reasoning.delta", seq: 4, messageId: ANSWER, index: 0, text: "Hmm" },
      { type: "text.delta", seq: 5, messageId: ANSWER, index: 1, text: "Hi" },
    ]);
    expect(view(result)).toEqual([
      {
        id: ANSWER,
        role: "assistant",
        status: "streaming",
        finishReason: null,
        usage: null,
        error: null,
        parts: [
          { type: "reasoning", text: "Hmm" },
          { type: "text", text: "Hi" },
        ],
      },
    ]);
  });

  test("a result before its tool message starts it as a tool message", () => {
    const result = run([
      {
        type: "tool.finished",
        seq: 3,
        messageId: TOOLS,
        callId: CALL,
        status: "denied",
        output: [{ type: "text", text: "Denied by the user." }],
        truncated: false,
      },
      { type: "message.started", seq: 4, messageId: NEXT, role: "assistant", windowStartId: null },
    ]);
    expect(view(result).map((message) => [message.id, message.role])).toEqual([
      [TOOLS, "tool"],
      [NEXT, "assistant"],
    ]);
  });

  test("a finish before the deltas keeps the status, and a repeated call replaces the first", () => {
    const call = {
      type: "tool.requested",
      messageId: ANSWER,
      callId: CALL,
      serverId: SERVER,
      toolName: "lookup",
      title: "Lookup",
      arguments: { page: 1 },
      needsApproval: false,
    } as const;
    const result = run([
      {
        type: "message.finished",
        seq: 1,
        messageId: ANSWER,
        status: "cancelled",
        finishReason: null,
        refusal: null,
        usage: null,
      },
      { ...call, seq: 2 },
      { ...call, seq: 3, arguments: { page: 2 } },
    ]);
    const [answer] = view(result);
    expect(answer?.status).toBe("cancelled");
    expect(answer?.parts).toHaveLength(1);
    expect(answer?.parts[0]).toMatchObject({ arguments: { page: 2 } });
  });
});

describe("the conversation on screen", () => {
  const history = [saved(USER, "user")];

  test("the user's new message shows until it is saved, then the run's messages", () => {
    const draft = {
      cut: null,
      message: {
        id: DRAFT_ID,
        role: "user" as const,
        status: "complete" as const,
        finishReason: null,
        parts: [{ type: "text" as const, text: "Weather?" }],
        usage: null,
        error: null,
      },
      after: USER,
    };
    const live = run(turn().slice(0, 6));
    expect(timeline(history, live, draft).map((message) => message.id)).toEqual([
      USER,
      DRAFT_ID,
      ANSWER,
    ]);
    // The saved messages now hold the new user message.
    const asked = [...history, saved(TOOLS, "user")];
    expect(timeline(asked, live, draft).map((message) => message.id)).toEqual([
      USER,
      TOOLS,
      ANSWER,
    ]);
  });

  test("a saved answer still streaming shows the run's copy; a final one wins", () => {
    const live = run(turn());
    const streaming = saved(ANSWER, "assistant", { status: "streaming", parts: [] });
    const [, answer] = timeline([...history, streaming], live, null);
    expect(answer?.parts).toHaveLength(3);
    const final = saved(ANSWER, "assistant");
    const [, kept, ...rest] = timeline([...history, final], live, null);
    expect(kept).toBe(final);
    expect(rest.map((message) => message.id)).toEqual([TOOLS, NEXT]);
  });

  test("an edit, regenerate, or retry hides the messages it replaces until they are gone", () => {
    const all = [saved(USER, "user"), saved(ANSWER, "assistant"), saved(TOOLS, "user")];
    const draft = { cut: ANSWER, message: null, after: USER };
    expect(timeline(all, null, draft).map((message) => message.id)).toEqual([USER]);
    // Saved again without the cut message: nothing is hidden.
    expect(timeline([saved(USER, "user")], null, draft).map((message) => message.id)).toEqual([
      USER,
    ]);
  });

  test("editing the first message hides the whole chat until the edit is saved", () => {
    const draft = {
      cut: USER,
      message: { ...saved(DRAFT_ID, "user"), id: DRAFT_ID },
      after: null,
    };
    const all = [saved(USER, "user"), saved(ANSWER, "assistant")];
    expect(timeline(all, null, draft).map((message) => message.id)).toEqual([DRAFT_ID]);
    const edited = [saved(NEXT, "user")];
    expect(timeline(edited, null, draft).map((message) => message.id)).toEqual([NEXT]);
  });
});

describe("decisions", () => {
  const waiting = saved(ANSWER, "assistant", {
    status: "awaiting_approval",
    finishReason: "tool_calls",
    parts: [
      {
        type: "toolCall",
        callId: CALL,
        serverId: SERVER,
        toolName: "delete_file",
        arguments: { path: "a.txt" },
        complete: true,
        approval: "pending",
        denyReason: null,
      },
      {
        type: "toolCall",
        callId: CALL_TWO,
        serverId: SERVER,
        toolName: "read_file",
        arguments: { path: "b.txt" },
        complete: true,
        approval: "pending",
        denyReason: null,
      },
    ],
  });

  test("only an answer awaiting approval has pending calls", () => {
    expect(pendingCalls(waiting).map((call) => call.callId)).toEqual([CALL, CALL_TWO]);
    expect(pendingCalls({ ...waiting, status: "complete" })).toEqual([]);
  });

  test("sent decisions approve or deny each call and end the wait", () => {
    const [user, answer] = decided([saved(USER, "user"), waiting], ANSWER, [
      { callId: CALL, decision: "deny", reason: "  Not that file.  " },
      { callId: CALL_TWO, decision: "approve", scope: "chat" },
    ]);
    expect(user?.id).toBe(USER);
    expect(answer?.status).toBe("complete");
    expect(answer?.parts).toEqual([
      expect.objectContaining({ approval: "denied", denyReason: "Not that file." }),
      expect.objectContaining({ approval: "approved", denyReason: null }),
    ]);
    const [, blank] = decided([saved(USER, "user"), waiting], ANSWER, [
      { callId: CALL, decision: "deny", reason: " " },
    ]);
    expect(blank?.parts[0]).toMatchObject({ approval: "denied", denyReason: null });
    expect(blank?.parts[1]).toMatchObject({ approval: "pending" });
  });
});

describe("the streaming routes", () => {
  type Call = { url: string; method: string; body: BodyInit | null | undefined };
  let calls: Call[] = [];

  afterEach(() => {
    calls = [];
    clearToken();
    resetTransport();
  });

  function serve(answer: () => Response): void {
    setToken("token");
    setTransport({
      server: () => "",
      fetch: async (input, init) => {
        calls.push({ url: input, method: init?.method ?? "GET", body: init?.body });
        return answer();
      },
    });
  }

  const stream = () =>
    new Response(`data: ${JSON.stringify({ type: "done", seq: 1 })}\n\n`, {
      headers: { "Content-Type": "text/event-stream" },
    });

  test("regenerate, retry, and decisions post to their routes and stream the turn", async () => {
    serve(stream);
    const seen: ChatStreamEvent[] = [];
    await regenerate(CHAT, (event) => seen.push(event));
    await retry(CHAT, (event) => seen.push(event));
    await decide(
      CHAT,
      ANSWER,
      { decisions: [{ callId: CALL, decision: "approve", scope: "once" }] },
      (event) => seen.push(event)
    );
    expect(calls.map((call) => [call.method, call.url])).toEqual([
      ["POST", `/api/chats/${CHAT}/regenerate`],
      ["POST", `/api/chats/${CHAT}/retry`],
      ["POST", `/api/chats/${CHAT}/messages/${ANSWER}/decisions`],
    ]);
    expect(calls[2]?.body).toBe(
      JSON.stringify({ decisions: [{ callId: CALL, decision: "approve", scope: "once" }] })
    );
    expect(seen).toHaveLength(3);
  });

  test("attach asks for the events after a seq and reports an idle chat", async () => {
    serve(stream);
    expect(await attach(CHAT, () => undefined, { after: 41 })).toBe(true);
    expect(await attach(CHAT, () => undefined)).toBe(true);
    expect(calls.map((call) => call.url)).toEqual([
      `/api/chats/${CHAT}/stream?after=41`,
      `/api/chats/${CHAT}/stream`,
    ]);
    serve(() => new Response(null, { status: 204 }));
    expect(await attach(CHAT, () => undefined)).toBe(false);
  });

  test("stop posts to the chat's stop route", async () => {
    serve(() => new Response(null, { status: 204 }));
    await stop(CHAT);
    expect(calls).toEqual([{ url: `/api/chats/${CHAT}/stop`, method: "POST", body: undefined }]);
  });
});

describe("tool servers a run could not reach", () => {
  test("the first answer names them, and the note says so", () => {
    const reply = "00000000-0000-7000-8000-0000000000aa";
    const started = run([
      {
        type: "message.started",
        seq: 1,
        messageId: reply,
        role: "assistant",
        windowStartId: null,
        unavailable: ["Docs", "Mail"],
      },
      { type: "message.started", seq: 2, messageId: reply, role: "assistant", windowStartId: null },
    ]);
    expect(started.unavailable).toEqual(["Docs", "Mail"]);
    expect(unreachableNote(started)).toBe(
      "Could not reach Docs, Mail. This answer has no tools from them."
    );
    expect(unreachableNote({ ...started, unavailable: ["Docs"] })).toBe(
      "Could not reach Docs. This answer has no tools from it."
    );
    expect(unreachableNote(run([]))).toBeNull();
    expect(unreachableNote(null)).toBeNull();
  });
});
