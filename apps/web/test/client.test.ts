import {
  type ChatStreamEvent,
  chatStreamEventSchema,
  healthResponseSchema,
  settingsBodySchema,
} from "@copilot/contracts";
import { afterEach, beforeEach, describe, expect, test } from "@rstest/core";
import {
  ApiError,
  apiFetch,
  apiRequest,
  apiStream,
  INVALID_RESPONSE,
  isAbort,
  NETWORK_ERROR,
  NO_SERVER,
  requestId,
  resetTransport,
  setTransport,
} from "@web/api/client";
import API from "@web/api/endpoints";
import { z } from "zod";

type Call = { url: string; method: string; headers: Headers; body: BodyInit | null | undefined };

let calls: Call[] = [];
let token: string | null = null;
let signedOut = 0;

/** Installs a transport whose `fetch` answers with `answer` and records each call. */
function serve(answer: (call: Call) => Response | Promise<Response>, server = ""): void {
  setTransport({
    server: () => server,
    token: () => token,
    signedOut: () => {
      signedOut++;
      token = null;
    },
    fetch: async (input, init) => {
      const call = {
        url: input,
        method: init?.method ?? "GET",
        headers: new Headers(init?.headers),
        body: init?.body,
      };
      calls.push(call);
      return answer(call);
    },
  });
}

function json(body: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json", ...headers },
  });
}

function stream(chunks: string[], status = 200): Response {
  const utf8 = new TextEncoder();
  return new Response(
    new ReadableStream({
      start(controller) {
        for (const chunk of chunks) controller.enqueue(utf8.encode(chunk));
        controller.close();
      },
    }),
    { status, headers: { "Content-Type": "text/event-stream" } }
  );
}

function event(value: ChatStreamEvent): string {
  return `data: ${JSON.stringify(value)}\n\n`;
}

async function failure(promise: Promise<unknown>): Promise<ApiError> {
  try {
    await promise;
  } catch (error) {
    if (error instanceof ApiError) return error;
    throw error;
  }
  throw new Error("expected the request to fail");
}

beforeEach(() => {
  calls = [];
  token = "secret-token";
  signedOut = 0;
});

afterEach(() => {
  resetTransport();
});

describe("apiFetch", () => {
  test("calls /api on the page's own origin with the bearer token and a request id", async () => {
    serve(() => json({ data: { instructions: "Be brief." } }));
    const body = await apiFetch(API.settings, settingsBodySchema);
    expect(body.data.instructions).toBe("Be brief.");
    const [call] = calls;
    expect(call?.url).toBe("/api/settings");
    expect(call?.method).toBe("GET");
    expect(call?.headers.get("Authorization")).toBe("Bearer secret-token");
    expect(call?.headers.get("Accept")).toBe("application/json");
    expect(call?.headers.get("X-Request-Id")).toMatch(/^[0-9a-f-]{36}$/);
    expect(call?.body).toBeUndefined();
  });

  test("sends no Authorization header when signed out", async () => {
    token = null;
    serve(() => json({ data: { instructions: "" } }));
    await apiFetch(API.settings, settingsBodySchema);
    expect(calls[0]?.headers.has("Authorization")).toBe(false);
  });

  test("sends the method and a JSON body", async () => {
    serve(() => json({ data: { instructions: "New" } }));
    await apiFetch(API.settings, settingsBodySchema, {
      method: "PATCH",
      body: { instructions: "New" },
    });
    expect(calls[0]?.method).toBe("PATCH");
    expect(calls[0]?.headers.get("Content-Type")).toBe("application/json");
    expect(calls[0]?.body).toBe('{"instructions":"New"}');
  });

  test("passes FormData through so the browser sets the multipart boundary", async () => {
    serve(() => new Response(null, { status: 204 }));
    const form = new FormData();
    form.append("file", new Blob(["hi"]), "a.txt");
    await apiRequest(API.chatFiles("c1"), { method: "POST", body: form });
    expect(calls[0]?.body).toBe(form);
    expect(calls[0]?.headers.has("Content-Type")).toBe(false);
  });

  test("encodes query parameters and leaves out empty ones", async () => {
    serve(() => json({ items: [], next: null }));
    await apiFetch(API.chats, z.unknown(), {
      query: { projectId: "p 1", pinned: true, cursor: null, limit: undefined, scheduled: false },
    });
    expect(calls[0]?.url).toBe("/api/chats?projectId=p+1&pinned=true&scheduled=false");
  });

  test("encodes ids in route paths", () => {
    expect(API.chatDecisions("a/b", "c?d")).toBe("/chats/a%2Fb/messages/c%3Fd/decisions");
    expect(API.auth.login).toBe("/auth/login");
  });

  test("parses a 204 answer as undefined", async () => {
    serve(() => new Response(null, { status: 204 }));
    await expect(apiFetch(API.chatStop("c1"), z.undefined(), { method: "POST" })).resolves.toBe(
      undefined
    );
  });

  test("turns an error body into ApiError with the server's message and fields", async () => {
    serve(() =>
      json(
        {
          error: "Username must be 3 to 32 characters long.",
          code: "VALIDATION_ERROR",
          field: "username",
          requestId: "req-1",
        },
        400
      )
    );
    const error = await failure(apiFetch(API.auth.register, z.unknown(), { method: "POST" }));
    expect(error.status).toBe(400);
    expect(error.message).toBe("Username must be 3 to 32 characters long.");
    expect(error.code).toBe("VALIDATION_ERROR");
    expect(error.field).toBe("username");
    expect(error.requestId).toBe("req-1");
  });

  test("reads the wait from details.retryAfterSecs, else from Retry-After", async () => {
    serve(() =>
      json(
        { error: "Too many requests.", code: "RATE_LIMITED", details: { retryAfterSecs: 42 } },
        429
      )
    );
    expect((await failure(apiFetch(API.auth.login, z.unknown()))).retryAfterSecs).toBe(42);

    serve(() => json({ error: "Slow down.", code: "RATE_LIMITED" }, 429, { "Retry-After": "7" }));
    expect((await failure(apiFetch(API.chats, z.unknown()))).retryAfterSecs).toBe(7);
  });

  test("keeps a generic message for a body that is not the error envelope", async () => {
    serve(() => new Response("<html>Bad gateway</html>", { status: 502 }));
    const error = await failure(apiFetch(API.chats, z.unknown()));
    expect(error.status).toBe(502);
    expect(error.message).toBe("The server answered with status 502.");
    expect(error.code).toBeUndefined();
  });

  test("reports an unreachable server as NETWORK_ERROR", async () => {
    serve(() => {
      throw new TypeError("Failed to fetch");
    });
    const error = await failure(apiFetch(API.chats, z.unknown()));
    expect(error.status).toBe(0);
    expect(error.code).toBe(NETWORK_ERROR);
    expect(error.message).toBe("The server cannot be reached.");
  });

  test("lets an abort through unchanged", async () => {
    serve(() => {
      throw new DOMException("The operation was aborted.", "AbortError");
    });
    const result = apiFetch(API.chats, z.unknown()).catch((error: unknown) => error);
    expect(isAbort(await result)).toBe(true);
  });

  test("rejects a success body that does not match the schema", async () => {
    serve(() => json({ data: { instructions: 7 } }));
    const error = await failure(apiFetch(API.settings, settingsBodySchema));
    expect(error.code).toBe(INVALID_RESPONSE);
    expect(error.status).toBe(200);
  });
});

describe("signing out on 401", () => {
  test("clears the session when the server rejects the current token", async () => {
    serve(() => json({ error: "Sign in again.", code: "UNAUTHORIZED" }, 401));
    await failure(apiFetch(API.chats, z.unknown()));
    expect(signedOut).toBe(1);
  });

  test("keeps the session for a wrong password", async () => {
    serve(() => json({ error: "Wrong username or password.", code: "INVALID_CREDENTIALS" }, 401));
    await failure(apiFetch(API.auth.login, z.unknown(), { method: "POST" }));
    expect(signedOut).toBe(0);
  });

  test("keeps a newer token that replaced the rejected one meanwhile", async () => {
    serve(() => {
      token = "newer-token";
      return json({ error: "Sign in again.", code: "UNAUTHORIZED" }, 401);
    });
    await failure(apiFetch(API.chats, z.unknown()));
    expect(signedOut).toBe(0);
    expect(token).toBe("newer-token");
  });

  test("does nothing for a request sent without a token", async () => {
    token = null;
    serve(() => json({ error: "Sign in.", code: "UNAUTHORIZED" }, 401));
    await failure(apiFetch(API.chats, z.unknown()));
    expect(signedOut).toBe(0);
  });
});

describe("runtime server URL", () => {
  test("builds requests from a stored native server URL", async () => {
    serve(() => json({ status: "healthy" }), "https://copilot.example/base");
    await apiFetch(API.settings, z.unknown());
    await apiFetch(API.health, healthResponseSchema, { root: true });
    expect(calls.map((call) => call.url)).toEqual([
      "https://copilot.example/base/api/settings",
      "https://copilot.example/base/health",
    ]);
    expect(calls[0]?.headers.get("Authorization")).toBe("Bearer secret-token");
  });

  test("never sends the token to another server", async () => {
    serve(() => json({ status: "healthy" }), "https://copilot.example");
    await apiFetch(API.health, healthResponseSchema, {
      root: true,
      server: "http://192.168.1.20:3000",
    });
    expect(calls[0]?.url).toBe("http://192.168.1.20:3000/health");
    expect(calls[0]?.headers.has("Authorization")).toBe(false);
  });

  test("refuses to send anything before a native build is connected", async () => {
    setTransport({ server: () => undefined, fetch: () => Promise.reject(new Error("unused")) });
    const error = await failure(apiFetch(API.chats, z.unknown()));
    expect(error.code).toBe(NO_SERVER);
  });
});

describe("requestId", () => {
  test("makes a version 4 UUID without crypto.randomUUID (plain http)", () => {
    const original = crypto.randomUUID;
    Object.defineProperty(crypto, "randomUUID", { value: undefined, configurable: true });
    try {
      const id = requestId();
      expect(id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
      expect(requestId()).not.toBe(id);
    } finally {
      Object.defineProperty(crypto, "randomUUID", { value: original, configurable: true });
    }
  });
});

describe("apiStream", () => {
  const messageId = "0195d6a4-1b2c-7d3e-8f40-123456789abc";

  test("posts the body and delivers each parsed event in order", async () => {
    serve(() =>
      stream([
        event({
          type: "message.started",
          seq: 0,
          messageId,
          role: "assistant",
          windowStartId: null,
        }),
        ": keep-alive\n\n",
        `data: {"type":"text.delta","seq":1,"messageId":"${messageId}","index":0,`,
        '"text":"Hi"}\n\n',
        event({ type: "done", seq: 2 }),
      ])
    );
    const events: ChatStreamEvent[] = [];
    const streamed = await apiStream(
      API.chatMessages("c1"),
      chatStreamEventSchema,
      (value) => events.push(value),
      { method: "POST", body: { text: "Hello", fileIds: [], editFrom: null } }
    );
    expect(streamed).toBe(true);
    expect(events.map((value) => value.type)).toEqual(["message.started", "text.delta", "done"]);
    expect(calls[0]?.headers.get("Accept")).toBe("text/event-stream");
    expect(calls[0]?.headers.get("Authorization")).toBe("Bearer secret-token");
    expect(calls[0]?.body).toBe('{"text":"Hello","fileIds":[],"editFrom":null}');
  });

  test("returns false when there is no run to attach to (204)", async () => {
    serve(() => new Response(null, { status: 204 }));
    const events: unknown[] = [];
    expect(
      await apiStream(API.chatStream("c1"), chatStreamEventSchema, (e) => events.push(e))
    ).toBe(false);
    expect(events).toEqual([]);
  });

  test("skips events a newer server added", async () => {
    serve(() =>
      stream(['data: {"type":"future.event","seq":0}\n\n', event({ type: "done", seq: 1 })])
    );
    const events: ChatStreamEvent[] = [];
    await apiStream(API.chatStream("c1"), chatStreamEventSchema, (value) => events.push(value));
    expect(events).toEqual([{ type: "done", seq: 1 }]);
  });

  test("fails on data that is not JSON", async () => {
    serve(() => stream(["data: {not json\n\n"]));
    const error = await failure(apiStream(API.chatStream("c1"), chatStreamEventSchema, () => {}));
    expect(error.code).toBe(INVALID_RESPONSE);
  });

  test("throws the error body before reading when the run is refused", async () => {
    serve(() => json({ error: "This chat is already answering.", code: "CHAT_BUSY" }, 422));
    const error = await failure(
      apiStream(API.chatMessages("c1"), chatStreamEventSchema, () => {}, { method: "POST" })
    );
    expect(error.status).toBe(422);
    expect(error.code).toBe("CHAT_BUSY");
  });

  test("stops reading when aborted", async () => {
    serve(() => stream([event({ type: "done", seq: 0 })]));
    const abort = new AbortController();
    const result = apiStream(API.chatStream("c1"), chatStreamEventSchema, () => abort.abort(), {
      signal: abort.signal,
    }).catch((error: unknown) => error);
    expect(isAbort(await result)).toBe(true);
  });
});
