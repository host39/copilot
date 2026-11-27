// The live events reader (`GET /events`): when it connects again and how long it waits, what it
// hands over, what each event makes stale, and the listeners.

import type { UserEvent } from "@copilot/contracts";
import { afterEach, describe, expect, test } from "@rstest/core";
import { ApiError, resetTransport, setTransport } from "@web/api/client";
import {
  CATCH_UP,
  CLOSED_RETRY_MS,
  MISSING_RETRY_MS,
  nextTry,
  onUserEvent,
  publish,
  staleKeys,
  type Wait,
  watchEvents,
} from "@web/api/events";
import { backoff } from "@web/api/retry";
import { keys } from "@web/api/routes/keys";
import { eventAction } from "@web/routes/chat/turn";

const CHAT = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708500";
const MESSAGE = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708501";
const SCHEDULE = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708400";

afterEach(() => {
  resetTransport();
});

/** An event stream body with these events, as the server writes them. */
function stream(...events: unknown[]): Response {
  const body = events.map((event) => `data: ${JSON.stringify(event)}\n\n`).join(": keep-alive\n\n");
  return new Response(body, { headers: { "Content-Type": "text/event-stream" } });
}

function failure(status: number, code = "INTERNAL_ERROR"): Response {
  return new Response(JSON.stringify({ error: "No.", code }), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

/** Answers each request with the next of `answers` (a thrown value is a network failure). */
function serve(answers: (() => Response)[]): { urls: string[]; accepts: (string | null)[] } {
  const seen = { urls: [] as string[], accepts: [] as (string | null)[] };
  setTransport({
    server: () => "",
    token: () => "token",
    signedOut: () => undefined,
    fetch: async (input, init) => {
      seen.urls.push(input);
      seen.accepts.push(new Headers(init?.headers).get("Accept"));
      const next = answers.shift();
      if (!next) throw new Error("no more answers");
      return next();
    },
  });
  return seen;
}

const unreachable = () => {
  throw new TypeError("Failed to fetch");
};

describe("when the reader connects again", () => {
  test("a closed stream right away, failures with backoff, a missing stream slowly", () => {
    expect(nextTry(undefined, 5)).toBe(CLOSED_RETRY_MS);
    const network = new ApiError(0, {
      error: "The server cannot be reached.",
      code: "NETWORK_ERROR",
    });
    expect(nextTry(network, 0)).toBe(1_000);
    expect(nextTry(network, 3)).toBe(8_000);
    expect(nextTry(network, 9)).toBe(15_000);
    expect(nextTry(new ApiError(503, { error: "Busy." }), 1)).toBe(2_000);
    expect(nextTry(new TypeError("cut"), 2)).toBe(backoff(2));
    for (const status of [404, 501, 403, 400]) {
      expect(nextTry(new ApiError(status, { error: "No." }), 0)).toBe(MISSING_RETRY_MS);
    }
  });

  test("never after the session ended, and after a rate limit's wait", () => {
    expect(nextTry(new ApiError(401, { error: "Sign in.", code: "UNAUTHORIZED" }), 0)).toBeNull();
    expect(nextTry(new ApiError(429, { error: "Slow down." }, 30), 0)).toBe(30_000);
    expect(nextTry(new ApiError(429, { error: "Slow down." }), 2)).toBe(4_000);
  });
});

describe("the events reader", () => {
  const started: UserEvent = { type: "run.started", chatId: CHAT };
  const finished: UserEvent = { type: "run.finished", chatId: CHAT, status: "complete" };

  test("hands over every event, reconnects after each ending, and stops when the session ends", async () => {
    const seen = serve([
      () => stream(started, { type: "chat.future", chatId: CHAT }, finished),
      unreachable,
      () => failure(503),
      () => stream({ type: "schedule.updated", scheduleId: SCHEDULE }),
      () => failure(401, "UNAUTHORIZED"),
    ]);
    const events: UserEvent[] = [];
    const opens: boolean[] = [];
    const waits: number[] = [];
    const wait: Wait = async (ms) => {
      waits.push(ms);
    };
    await watchEvents({
      onEvent: (event) => events.push(event),
      onOpen: (again) => opens.push(again),
      signal: new AbortController().signal,
      wakes: () => () => undefined,
      wait,
    });

    expect(seen.urls).toEqual(Array(5).fill("/api/events"));
    expect(seen.accepts.every((accept) => accept === "text/event-stream")).toBe(true);
    // The event this client does not know is skipped.
    expect(events).toEqual([started, finished, { type: "schedule.updated", scheduleId: SCHEDULE }]);
    expect(opens).toEqual([false, true]);
    // Closed, unreachable (first failure), 503 (second failure), closed again.
    expect(waits).toEqual([CLOSED_RETRY_MS, 1_000, 2_000, CLOSED_RETRY_MS]);
  });

  test("an open stream resets the backoff", async () => {
    serve([
      unreachable,
      unreachable,
      () => stream(),
      unreachable,
      () => failure(401, "UNAUTHORIZED"),
    ]);
    const waits: number[] = [];
    await watchEvents({
      onEvent: () => undefined,
      signal: new AbortController().signal,
      wakes: () => () => undefined,
      wait: async (ms) => {
        waits.push(ms);
      },
    });
    expect(waits).toEqual([1_000, 2_000, CLOSED_RETRY_MS, 1_000]);
  });

  test("a wake cuts the wait short, and aborting stops the reader during a wait", async () => {
    serve([unreachable, unreachable, unreachable]);
    let wake: () => void = () => undefined;
    const controller = new AbortController();
    let waits = 0;
    const done = watchEvents({
      onEvent: () => undefined,
      signal: controller.signal,
      wakes: (callback) => {
        wake = callback;
        return () => undefined;
      },
      // Waits until woken: no timers in the test.
      wait: (_ms, woken) =>
        new Promise((resolve) => {
          waits += 1;
          woken.addEventListener("abort", () => resolve(), { once: true });
        }),
    });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(waits).toBe(1);
    wake();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(waits).toBe(2);
    controller.abort();
    await done;
    expect(waits).toBe(2);
  });

  test("aborting while a stream is open stops reading without another try", async () => {
    const controller = new AbortController();
    setTransport({
      server: () => "",
      token: () => "token",
      fetch: async (_input, init) =>
        new Response(
          new ReadableStream({
            start(source) {
              source.enqueue(new TextEncoder().encode(`data: ${JSON.stringify(started)}\n\n`));
              init?.signal?.addEventListener("abort", () =>
                source.error(new DOMException("x", "AbortError"))
              );
            },
          }),
          { headers: { "Content-Type": "text/event-stream" } }
        ),
    });
    const events: UserEvent[] = [];
    const waits: number[] = [];
    const done = watchEvents({
      onEvent: (event) => {
        events.push(event);
        controller.abort();
      },
      signal: controller.signal,
      wakes: () => () => undefined,
      wait: async (ms) => {
        waits.push(ms);
      },
    });
    await done;
    expect(events).toEqual([started]);
    expect(waits).toEqual([]);
  });
});

describe("what an event makes stale", () => {
  const lists = [{ queryKey: ["chats", "list"] }, { queryKey: keys.runs }];

  test("chat lists and the Scheduled count for chats and runs; the changed chat alone", () => {
    expect(staleKeys({ type: "chat.updated", chatId: CHAT })).toEqual([
      ...lists,
      { queryKey: keys.chat(CHAT), exact: true },
    ]);
    for (const event of [
      { type: "run.started", chatId: CHAT },
      { type: "run.finished", chatId: CHAT, status: "error" },
      { type: "approval.pending", chatId: CHAT, messageId: MESSAGE },
    ] as UserEvent[]) {
      expect(staleKeys(event)).toEqual(lists);
    }
  });

  test("never a chat's messages, which would mark it read", () => {
    const events: UserEvent[] = [
      { type: "chat.updated", chatId: CHAT },
      { type: "approval.pending", chatId: CHAT, messageId: MESSAGE },
    ];
    for (const event of events) {
      for (const filter of staleKeys(event)) {
        const prefix = !filter.exact;
        const covers =
          prefix && keys.chatMessages(CHAT).join("/").startsWith(filter.queryKey.join("/"));
        expect(covers).toBe(false);
      }
    }
  });

  test("schedule lists and the schedule; after a dropped stream, every list", () => {
    expect(staleKeys({ type: "schedule.updated", scheduleId: SCHEDULE })).toEqual([
      { queryKey: ["schedules", "list"] },
      { queryKey: keys.schedule(SCHEDULE) },
    ]);
    expect(CATCH_UP).toEqual([...lists, { queryKey: ["schedules", "list"] }]);
  });
});

describe("listeners", () => {
  test("hear events until they unsubscribe, and one that throws does not stop the others", () => {
    const heard: string[] = [];
    const stopFirst = onUserEvent(() => {
      heard.push("first");
      throw new Error("boom");
    });
    const stopSecond = onUserEvent((event) => heard.push(event.type));
    publish({ type: "chat.updated", chatId: CHAT });
    stopFirst();
    publish({ type: "run.started", chatId: CHAT });
    stopSecond();
    publish({ type: "run.started", chatId: CHAT });
    expect(heard).toEqual(["first", "chat.updated", "run.started"]);
  });
});

describe("a chat page and the live events", () => {
  const other = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708599";

  test("attaches to a run another device starts, and rereads a run that ended or waits", () => {
    expect(eventAction({ type: "run.started", chatId: CHAT }, CHAT, false)).toBe("attach");
    expect(
      eventAction({ type: "run.finished", chatId: CHAT, status: "complete" }, CHAT, false)
    ).toBe("refresh");
    expect(
      eventAction({ type: "approval.pending", chatId: CHAT, messageId: MESSAGE }, CHAT, false)
    ).toBe("refresh");
    expect(eventAction({ type: "chat.updated", chatId: CHAT }, CHAT, false)).toBeNull();
  });

  test("ignores other chats, schedules, and everything while it follows a stream", () => {
    expect(eventAction({ type: "run.started", chatId: other }, CHAT, false)).toBeNull();
    expect(eventAction({ type: "schedule.updated", scheduleId: SCHEDULE }, CHAT, false)).toBeNull();
    expect(eventAction({ type: "run.started", chatId: CHAT }, CHAT, true)).toBeNull();
    expect(
      eventAction({ type: "run.finished", chatId: CHAT, status: "error" }, CHAT, true)
    ).toBeNull();
  });
});
