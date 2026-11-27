// The first-run setup: its steps, saving and fixing the connection, the model the new chat opens
// with, and where a sign-in lands.

import type { ModelResponse, ProviderResponse, UserResponse } from "@copilot/contracts";
import { afterEach, describe, expect, test } from "@rstest/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { resetTransport, setTransport } from "@web/api/client";
import { clearToken, setToken } from "@web/auth/token";
import HomePage from "@web/routes/home/page";
import path from "@web/routes/path";
import { CHOICES, type Choice, providerSchema } from "@web/routes/settings/providers/presets";
import WelcomePage from "@web/routes/welcome/page";
import {
  firstModel,
  initialSetup,
  keepsKey,
  landing,
  type SetupEvent,
  type SetupState,
  saveConnection,
  setupReducer,
} from "@web/routes/welcome/setup";
import type { ReactNode } from "react";
import { renderToString } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";

const AT = "2026-11-26T10:00:00Z";
const ID = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708100";
const MODEL = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708101";
const OTHER = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708102";

const pick = (id: string) => CHOICES.find((item) => item.id === id) as Choice;
const report = { added: 3, updated: 0, unavailable: 0, enabled: 3 };

const provider: ProviderResponse = {
  id: ID,
  kind: "openai",
  preset: "openai",
  name: "OpenAI",
  baseUrl: "https://api.openai.com/v1",
  enabled: true,
  key: "set",
  createdAt: AT,
  updatedAt: AT,
};

const capabilities = {
  vision: false,
  pdf: false,
  tools: true,
  audioIn: false,
  audioOut: false,
  reasoning: false,
  reasoningLevels: [],
  defaultReasoning: null,
};

function model(id: string, displayName: string, enabled: boolean): ModelResponse {
  return {
    id,
    providerId: ID,
    providerName: "OpenAI",
    kind: "openai",
    preset: "openai",
    remoteId: displayName.toLowerCase(),
    displayName,
    enabled,
    available: true,
    contextWindow: null,
    maxOutputTokens: null,
    capabilities,
    detected: capabilities,
    overrides: {
      vision: null,
      pdf: null,
      tools: null,
      audioIn: null,
      audioOut: null,
      reasoning: null,
    },
    locks: [],
    reasoningOffWithTools: false,
    createdAt: AT,
    updatedAt: AT,
  };
}

const run = (events: SetupEvent[], from: SetupState = initialSetup) =>
  events.reduce(setupReducer, from);

type Call = { url: string; method: string; body: unknown };
let calls: Call[] = [];

function serve(answer: (call: Call) => Response): void {
  setTransport({
    server: () => "",
    token: () => "token",
    fetch: async (url, init) => {
      const call = {
        url,
        method: init?.method ?? "GET",
        body: typeof init?.body === "string" ? JSON.parse(init.body) : undefined,
      };
      calls.push(call);
      return answer(call);
    },
  });
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

afterEach(() => {
  calls = [];
  clearToken();
  resetTransport();
});

describe("the setup steps", () => {
  test("saving the connection moves to the test, which runs at once", () => {
    const state = run([
      { type: "pick", pick: pick("ollama") },
      { type: "saved", provider },
    ]);
    expect(state.step).toBe(1);
    expect(state.pick.id).toBe("ollama");
    expect(state.test).toEqual({ status: "running" });
  });

  test("continues to the models only after a passed test", () => {
    const failed = run([
      { type: "saved", provider },
      { type: "failed", problem: "Key rejected." },
      { type: "next" },
    ]);
    expect(failed.step).toBe(1);
    expect(failed.test).toEqual({ status: "failed", problem: "Key rejected." });

    const passed = run([{ type: "saved", provider }, { type: "tested", report }, { type: "next" }]);
    expect(passed.step).toBe(2);
  });

  test("going back from the models keeps the test; from the test it edits the connection", () => {
    const models = run([{ type: "saved", provider }, { type: "tested", report }, { type: "next" }]);
    const test = run([{ type: "back" }], models);
    expect(test.step).toBe(1);
    expect(test.test.status).toBe("passed");
    const connect = run([{ type: "back" }], test);
    expect(connect.step).toBe(0);
    expect(connect.test).toEqual({ status: "idle" });
    expect(connect.provider).toEqual(provider);
    expect(run([{ type: "back" }], connect).step).toBe(0);
  });

  test("ignores test results and preset changes outside their step", () => {
    expect(run([{ type: "tested", report }])).toEqual(initialSetup);
    const testing = run([{ type: "saved", provider }]);
    expect(run([{ type: "pick", pick: pick("gemini") }], testing).pick.id).toBe("openai");
  });

  test("a saved key of the same preset may stay", () => {
    const back = run([
      { type: "saved", provider },
      { type: "failed", problem: "x" },
      { type: "back" },
    ]);
    expect(keepsKey(back)).toBe(true);
    expect(keepsKey(run([{ type: "pick", pick: pick("gemini") }], back))).toBe(false);
    expect(keepsKey({ ...back, provider: { ...provider, key: "missing" } })).toBe(false);
    expect(keepsKey(initialSetup)).toBe(false);

    const schema = providerSchema(pick("openai"), true);
    expect(schema.parse({ name: "", baseUrl: "", apiKey: "" })).toEqual({
      kind: "openai",
      preset: "openai",
      name: "OpenAI",
    });
  });
});

describe("saving the connection", () => {
  test("creates it the first time", async () => {
    serve(() => json({ data: provider }, 201));
    await saveConnection(undefined, { kind: "openai", preset: "openai", name: "OpenAI" });
    expect(calls.map((call) => `${call.method} ${call.url}`)).toEqual(["POST /api/providers"]);
  });

  test("updates the saved one with a new key, without a second connection", async () => {
    serve(() => json({ data: provider }));
    await saveConnection(provider, {
      kind: "openai",
      preset: "openai",
      name: "OpenAI",
      apiKey: "sk-2",
    });
    expect(calls).toEqual([
      { url: `/api/providers/${ID}`, method: "PATCH", body: { apiKey: "sk-2" } },
    ]);
  });

  test("sends nothing when nothing changed", async () => {
    serve(() => json({ data: provider }));
    expect(
      await saveConnection(provider, {
        kind: "openai",
        preset: "openai",
        name: "OpenAI",
        baseUrl: provider.baseUrl,
      })
    ).toEqual(provider);
    expect(calls).toEqual([]);
  });

  test("replaces it when the preset changed", async () => {
    serve((call) =>
      call.method === "DELETE" ? new Response(null, { status: 204 }) : json({ data: provider }, 201)
    );
    await saveConnection(provider, { kind: "anthropic", preset: "anthropic", name: "Anthropic" });
    expect(calls.map((call) => `${call.method} ${call.url}`)).toEqual([
      `DELETE /api/providers/${ID}`,
      "POST /api/providers",
    ]);
  });
});

describe("after the setup", () => {
  test("the new chat opens with the first model turned on", () => {
    expect(
      firstModel([
        model(MODEL, "Zeta", true),
        model(OTHER, "Alpha", false),
        model(ID, "Beta", true),
      ])?.displayName
    ).toBe("Beta");
    expect(firstModel([model(MODEL, "Zeta", false)])).toBeUndefined();
  });

  test("a sign-in without a provider lands in the setup", async () => {
    serve(() => json({ items: [], total: 0 }));
    expect(await landing("/chats/1")).toBe(path.welcome);
    serve(() => json({ items: [provider], total: 1 }));
    expect(await landing("/chats/1")).toBe("/chats/1");
    serve(() => json({ error: "Down." }, 503));
    expect(await landing(path.home)).toBe(path.home);
  });
});

describe("the setup screens", () => {
  const user: UserResponse = {
    id: "0195d6a4-1b2c-7d3e-8f40-123456789abc",
    username: "ada",
    displayName: "Ada",
    role: "administrator",
    disabled: false,
    createdAt: AT,
    updatedAt: AT,
  };

  function render(
    page: ReactNode,
    entry: string | { pathname: string; state: unknown },
    data: [readonly unknown[], unknown][] = []
  ) {
    const client = new QueryClient();
    setToken("token");
    client.setQueryData(["auth", "me", "token"], user);
    for (const [key, value] of data) client.setQueryData(key, value);
    return renderToString(
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={[entry]}>{page}</MemoryRouter>
      </QueryClientProvider>
    );
  }

  test("starts with the presets and a key, and can be skipped", () => {
    const html = render(<WelcomePage />, path.welcome);
    expect(html).toContain("Welcome, Ada");
    expect(html).toContain('aria-current="step"');
    expect(html).toContain("OpenAI-compatible");
    expect(html).toContain('name="apiKey"');
    expect(html).not.toContain('name="name"');
    expect(html).toContain("Skip for now");
  });

  test("the new chat screen picks the model the setup turned on", () => {
    const html = render(<HomePage />, { pathname: path.home, state: { modelId: OTHER } }, [
      [["models"], [model(MODEL, "First", true), model(OTHER, "Second", true)]],
    ]);
    expect(html).toContain(">Second</span>");
    expect(html).not.toContain(">First</span>");
  });
});
