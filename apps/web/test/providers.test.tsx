// Settings, Providers: the add form and its test, the connection form, the model list and editor,
// the delete confirmation, and the screens.

import type {
  ModelResponse,
  ProviderDetailsResponse,
  ProviderResponse,
  UserResponse,
} from "@copilot/contracts";
import { afterEach, describe, expect, test } from "@rstest/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { resetTransport, setTransport } from "@web/api/client";
import { keys } from "@web/api/routes/keys";
import { clearToken, setToken } from "@web/auth/token";
import { fieldErrors } from "@web/forms";
import path from "@web/routes/path";
import {
  choosesApi,
  connectionSchema,
  dropsKey,
  sameOrigin,
} from "@web/routes/settings/provider/connection";
import {
  filterModels,
  modelSchema,
  shortTokens,
  usageText,
} from "@web/routes/settings/provider/models";
import ProviderPage from "@web/routes/settings/provider/page";
import ProvidersPage from "@web/routes/settings/providers/page";
import {
  CHOICES,
  type Choice,
  choice,
  connect,
  providerSchema,
  refreshSummary,
} from "@web/routes/settings/providers/presets";
import { renderToString } from "react-dom/server";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import type { z } from "zod";

const AT = "2026-11-26T10:00:00Z";
const ID = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708100";

const pick = (id: string) => CHOICES.find((item) => item.id === id) as Choice;

const provider: ProviderResponse = {
  id: ID,
  kind: "openai",
  preset: "openai",
  api: "responses",
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

const none = {
  vision: null,
  pdf: null,
  tools: null,
  audioIn: null,
  audioOut: null,
  reasoning: null,
};

function model(remoteId: string, extra: Partial<ModelResponse> = {}): ModelResponse {
  return {
    id: `0196f6a4-7c1e-7d2a-9b3c-${remoteId.length.toString().padStart(12, "0")}`,
    providerId: ID,
    providerName: "OpenAI",
    kind: "openai",
    preset: "openai",
    remoteId,
    displayName: remoteId,
    enabled: true,
    available: true,
    contextWindow: null,
    maxOutputTokens: null,
    capabilities,
    detected: capabilities,
    overrides: none,
    locks: [],
    reasoningOffWithTools: false,
    createdAt: AT,
    updatedAt: AT,
    ...extra,
  };
}

/** The field errors of a failed parse, or the parsed value. */
function parse<S extends z.ZodType>(schema: S, input: unknown) {
  const parsed = schema.safeParse(input);
  return parsed.success ? { value: parsed.data } : { errors: fieldErrors(parsed.error) };
}

afterEach(() => {
  clearToken();
  resetTransport();
});

describe("the add-provider form", () => {
  test("hosted providers need a key and never send an address", () => {
    expect(parse(providerSchema(pick("openai")), { name: "", baseUrl: "", apiKey: "" })).toEqual({
      errors: { apiKey: "Paste your OpenAI API key." },
    });
    expect(
      parse(providerSchema(pick("anthropic")), {
        name: " ",
        baseUrl: "https://elsewhere.example.com",
        apiKey: " sk-ant ",
      })
    ).toEqual({
      value: { kind: "anthropic", preset: "anthropic", name: "Anthropic", apiKey: "sk-ant" },
    });
  });

  test("compatible servers need an http address; the key is optional", () => {
    const schema = providerSchema(pick("custom-openai"));
    expect(parse(schema, { name: "", baseUrl: "", apiKey: "" })).toEqual({
      errors: { baseUrl: "Enter the server address." },
    });
    expect(parse(schema, { name: "", baseUrl: "ftp://files.example.com", apiKey: "" })).toEqual({
      errors: { baseUrl: "Enter a full http:// or https:// address." },
    });
    expect(parse(schema, { name: "vLLM", baseUrl: "http://10.0.0.5:8000/v1", apiKey: "" })).toEqual(
      {
        value: {
          kind: "openai",
          preset: "custom",
          name: "vLLM",
          baseUrl: "http://10.0.0.5:8000/v1",
          api: "chat_completions",
        },
      }
    );
  });

  test("OpenAI and compatible servers choose an API; other presets send none", () => {
    const filled = { name: "", baseUrl: "https://llm.example.com/v1", apiKey: "sk-1" };
    expect(parse(providerSchema(pick("openai")), filled)).toEqual({
      value: { kind: "openai", preset: "openai", name: "OpenAI", apiKey: "sk-1", api: "responses" },
    });
    expect(
      parse(providerSchema(pick("openai")), { ...filled, api: "chat_completions" })
    ).toMatchObject({ value: { api: "chat_completions" } });
    expect(
      parse(providerSchema(pick("custom-openai")), { ...filled, api: "responses" })
    ).toMatchObject({ value: { preset: "custom", api: "responses" } });
    expect(parse(providerSchema(pick("openai")), { ...filled, api: "completions" })).toEqual({
      errors: { api: expect.any(String) },
    });
    for (const id of ["gemini", "openrouter", "anthropic", "custom-anthropic"]) {
      const result = parse(providerSchema(pick(id)), { ...filled, api: "responses" });
      expect(result.value).not.toHaveProperty("api");
    }
    expect(
      parse(providerSchema(pick("ollama")), { ...filled, api: "responses" }).value
    ).not.toHaveProperty("api");
  });

  test("Ollama takes an optional address and never sends a key", () => {
    const schema = providerSchema(pick("ollama"));
    expect(parse(schema, { name: "", baseUrl: "", apiKey: "secret" })).toEqual({
      value: { kind: "openai", preset: "ollama", name: "Ollama" },
    });
  });

  test("names have at most 100 characters", () => {
    const result = parse(providerSchema(pick("ollama")), {
      name: "x".repeat(101),
      baseUrl: "",
      apiKey: "",
    });
    expect(result).toEqual({ errors: { name: "Use at most 100 characters." } });
  });

  test("an existing connection maps back to its preset", () => {
    expect(choice("openai", "gemini").label).toBe("Gemini");
    expect(choice("anthropic", "custom").label).toBe("Anthropic-compatible");
    expect(choice("anthropic", "openrouter").label).toBe("Anthropic-compatible");
  });
});

describe("adding and testing a connection", () => {
  type Call = { url: string; method: string };
  let calls: Call[] = [];

  function serve(refresh: Response): void {
    setTransport({
      server: () => "",
      token: () => "token",
      fetch: async (url, init) => {
        calls.push({ url, method: init?.method ?? "GET" });
        if (url.endsWith("/refresh")) return refresh;
        return new Response(JSON.stringify({ data: provider }), {
          status: 201,
          headers: { "Content-Type": "application/json" },
        });
      },
    });
  }

  afterEach(() => {
    calls = [];
  });

  test("saves the connection, then lists its models", async () => {
    serve(
      new Response(JSON.stringify({ data: { added: 4, updated: 0, unavailable: 0, enabled: 4 } }), {
        headers: { "Content-Type": "application/json" },
      })
    );
    const connected = await connect({ kind: "openai", preset: "openai", name: "OpenAI" });
    expect(calls).toEqual([
      { url: "/api/providers", method: "POST" },
      { url: `/api/providers/${ID}/refresh`, method: "POST" },
    ]);
    expect(connected.refresh?.added).toBe(4);
    expect(connected.problem).toBeUndefined();
  });

  test("a failed test keeps the connection and says why", async () => {
    serve(
      new Response(JSON.stringify({ error: "Key rejected.", code: "PROVIDER_AUTH" }), {
        status: 502,
        headers: { "Content-Type": "application/json" },
      })
    );
    const connected = await connect({ kind: "openai", preset: "openai", name: "OpenAI" });
    expect(connected.provider.id).toBe(ID);
    expect(connected.problem).toBe("Key rejected.");
  });

  test("summarizes what a refresh found", () => {
    expect(refreshSummary({ added: 4, updated: 0, unavailable: 0, enabled: 4 })).toBe(
      "Found 4 models. All are enabled."
    );
    expect(refreshSummary({ added: 1, updated: 0, unavailable: 0, enabled: 1 })).toBe(
      "Found 1 model. It is enabled."
    );
    expect(refreshSummary({ added: 30, updated: 0, unavailable: 0, enabled: 0 })).toBe(
      "Found 30 models. Choose the ones to enable."
    );
    expect(refreshSummary({ added: 2, updated: 5, unavailable: 1, enabled: 2 })).toBe(
      "Found 7 models, 2 new, 1 no longer listed. 2 new models enabled."
    );
    expect(refreshSummary({ added: 0, updated: 0, unavailable: 0, enabled: 0 })).toBe(
      "Found 0 models."
    );
  });
});

describe("the connection form", () => {
  const values = { name: "OpenAI", baseUrl: "https://api.openai.com/v1", apiKey: "" };

  test("sends only what changed, and a key only when one is typed", () => {
    expect(parse(connectionSchema(provider), values)).toEqual({ value: {} });
    expect(
      parse(connectionSchema(provider), { ...values, name: " Work ", apiKey: " sk-2 " })
    ).toEqual({ value: { name: "Work", apiKey: "sk-2" } });
  });

  test("sends the API only when it changed, and only where the preset chooses one", () => {
    expect(parse(connectionSchema(provider), { ...values, api: "responses" })).toEqual({
      value: {},
    });
    expect(parse(connectionSchema(provider), { ...values, api: "chat_completions" })).toEqual({
      value: { api: "chat_completions" },
    });
    const gemini: ProviderResponse = { ...provider, preset: "gemini", api: "chat_completions" };
    expect(choosesApi(gemini)).toBe(false);
    expect(choosesApi({ ...provider, preset: "custom" })).toBe(true);
    expect(choosesApi({ ...provider, kind: "anthropic", preset: "custom", api: null })).toBe(false);
    expect(
      parse(connectionSchema(gemini), {
        ...values,
        baseUrl: gemini.baseUrl,
        api: "responses",
      })
    ).toEqual({ value: {} });
  });

  test("needs a name and an http address", () => {
    expect(parse(connectionSchema(provider), { name: "", baseUrl: "api", apiKey: "" })).toEqual({
      errors: { name: "Enter a name.", baseUrl: "Enter a full http:// or https:// address." },
    });
  });

  test("a new origin without a new key drops the stored key", () => {
    expect(sameOrigin("https://api.openai.com/v1", "https://api.openai.com/v2")).toBe(true);
    expect(sameOrigin("https://api.openai.com", "http://api.openai.com")).toBe(false);
    expect(dropsKey(provider, { baseUrl: "https://gateway.example.com/v1" })).toBe(true);
    expect(dropsKey(provider, { baseUrl: "https://gateway.example.com/v1", apiKey: "k" })).toBe(
      false
    );
    expect(dropsKey(provider, { baseUrl: "https://api.openai.com/v2" })).toBe(false);
    expect(dropsKey({ ...provider, key: "missing" }, { baseUrl: "https://x.example.com" })).toBe(
      false
    );
  });
});

describe("the model list", () => {
  const models = [
    model("gpt-mini", { enabled: false }),
    model("old-model", { available: false }),
    model("GPT-Large"),
    model("claude"),
  ];

  test("matches every word in the name or id, ignoring case", () => {
    const names = (query: string) =>
      filterModels(models, query, "all").map((item) => item.remoteId);
    expect(names("gpt")).toEqual(["GPT-Large", "gpt-mini"]);
    expect(names("GPT large")).toEqual(["GPT-Large"]);
    expect(names("nothing")).toEqual([]);
  });

  test("filters by state and lists models no longer offered last", () => {
    expect(filterModels(models, "", "enabled").map((item) => item.remoteId)).toEqual([
      "claude",
      "GPT-Large",
      "old-model",
    ]);
    expect(filterModels(models, "", "disabled").map((item) => item.remoteId)).toEqual(["gpt-mini"]);
  });

  test("shortens token counts", () => {
    expect(shortTokens(950)).toBe("950");
    expect(shortTokens(128_000)).toBe("128K");
    expect(shortTokens(1_000_000)).toBe("1M");
    expect(shortTokens(1_500_000)).toBe("1.5M");
  });
});

describe("the model editor", () => {
  const gpt = model("gpt", { contextWindow: 128_000, maxOutputTokens: 16_000 });
  const same = {
    displayName: "gpt",
    contextWindow: "128000",
    maxOutputTokens: "16000",
    vision: "default",
    tools: "default",
  };

  test("an unchanged form sends nothing", () => {
    expect(parse(modelSchema(gpt), same)).toEqual({ value: {} });
  });

  test("sends changed names and limits; an empty limit clears it", () => {
    expect(
      parse(modelSchema(gpt), {
        ...same,
        displayName: " GPT ",
        contextWindow: "200,000",
        maxOutputTokens: "",
      })
    ).toEqual({ value: { displayName: "GPT", contextWindow: 200_000, maxOutputTokens: null } });
  });

  test("limits are positive whole numbers, and the output fits the window", () => {
    expect(parse(modelSchema(gpt), { ...same, contextWindow: "0" })).toEqual({
      errors: { contextWindow: "The context window must be greater than zero." },
    });
    expect(parse(modelSchema(gpt), { ...same, maxOutputTokens: "lots" })).toEqual({
      errors: { maxOutputTokens: "Enter a whole number of tokens." },
    });
    expect(parse(modelSchema(gpt), { ...same, maxOutputTokens: "128000" })).toEqual({
      errors: { maxOutputTokens: "Max output tokens must be less than the context window." },
    });
    expect(parse(modelSchema(gpt), { ...same, displayName: "" })).toEqual({
      errors: { displayName: "Enter a name." },
    });
  });

  test("overrides change only the flags the user picked", () => {
    expect(parse(modelSchema(gpt), { ...same, vision: "on", tools: "off" })).toEqual({
      value: { capabilityOverrides: { ...none, vision: true, tools: false } },
    });
    const overridden = model("gpt", {
      contextWindow: 128_000,
      maxOutputTokens: 16_000,
      overrides: { ...none, vision: true },
    });
    expect(parse(modelSchema(overridden), same)).toEqual({
      value: { capabilityOverrides: none },
    });
  });

  test("never overrides a locked flag", () => {
    const locked = model("gpt-6", {
      locks: [{ capability: "tools", reason: "Needs the Responses adapter." }],
    });
    expect(
      parse(modelSchema(locked), {
        displayName: "gpt-6",
        contextWindow: "",
        maxOutputTokens: "",
        tools: "on",
        pdf: "on",
      })
    ).toEqual({ value: { capabilityOverrides: { ...none, pdf: true } } });
  });
});

describe("the delete confirmation", () => {
  test("says what loses its model", () => {
    expect(usageText({ chats: 0, projects: 0, schedules: 0 })).toBe(
      "No chat, project, or schedule uses its models."
    );
    expect(usageText({ chats: 3, projects: 1, schedules: 2 })).toBe(
      "Its models are used by 3 chats, 1 project, and 2 schedules. Chats and projects keep their messages and settings, without a model until you pick another. Schedules that run one of them are turned off."
    );
    expect(usageText({ chats: 0, projects: 0, schedules: 1 })).toBe(
      "Its models are used by 1 schedule. Schedules that run one of them are turned off."
    );
  });
});

describe("the provider screens", () => {
  const user: UserResponse = {
    id: "0195d6a4-1b2c-7d3e-8f40-123456789abc",
    username: "ada",
    displayName: "Ada",
    role: "user",
    disabled: false,
    createdAt: AT,
    updatedAt: AT,
  };

  function render(
    entry: string | { pathname: string; state: unknown },
    data: [readonly unknown[], unknown][]
  ) {
    const client = new QueryClient();
    setToken("token");
    client.setQueryData(["auth", "me", "token"], user);
    for (const [key, value] of data) client.setQueryData(key, value);
    return renderToString(
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={[entry]}>
          <Routes>
            <Route path={path.providers} element={<ProvidersPage />} />
            <Route path={path.providerNew} element={<ProviderPage />} />
            <Route path={path.provider} element={<ProviderPage />} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );
  }

  const details: ProviderDetailsResponse = {
    ...provider,
    usage: { chats: 2, projects: 0, schedules: 0 },
  };

  test("lists connections with their preset, address, and problems", () => {
    const html = render(path.providers, [
      [
        keys.providers,
        [
          { ...provider, name: "Work", key: "unreadable" },
          { ...provider, id: "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708109", enabled: false },
        ],
      ],
    ]);
    expect(html).toContain("Work");
    expect(html).toContain("OpenAI · api.openai.com/v1");
    expect(html).toContain("Key unreadable");
    expect(html).toContain(">Off<");
    expect(html).toContain(`href="${path.providerNew}"`);
  });

  test("without connections it says what can be added", () => {
    expect(render(path.providers, [[keys.providers, []]])).toContain("No providers yet");
  });

  test("the add screen asks for a hosted provider's key", () => {
    const html = render(path.providerNew, []);
    expect(html).toContain("Add Provider");
    expect(html).toContain("Anthropic-compatible");
    expect(html).toContain('name="apiKey"');
    expect(html).not.toContain('name="baseUrl"');
    expect(html).toContain('name="api"');
    expect(html).toMatch(/name="api" checked="" value="responses"/);
    expect(html).toContain("Chat Completions");
  });

  test("a connection shows its settings, its models, and delete", () => {
    const html = render(`/settings/providers/${ID}`, [
      [keys.provider(ID), details],
      [
        keys.providerModels(ID),
        [model("gpt", { contextWindow: 128_000 }), model("legacy", { available: false })],
      ],
    ]);
    expect(html).toContain('value="https://api.openai.com/v1"');
    expect(html).toContain("Key stored");
    expect(html).toContain("Responses API");
    expect(html).toMatch(/name="api" checked="" value="responses"/);
    expect(html).toContain("Leave empty to keep the stored key");
    expect(html).toContain("2 of 2 turned on");
    expect(html).toContain("Tool use · 128K context");
    expect(html).toContain("No longer listed");
    expect(html).toContain('role="switch"');
    expect(html).toContain("Its models are used by 2 chats.");
  });

  test("shows the first test's result after adding", () => {
    const entry = (state: unknown) => ({ pathname: `/settings/providers/${ID}`, state });
    const data: [readonly unknown[], unknown][] = [
      [keys.provider(ID), details],
      [keys.providerModels(ID), []],
    ];
    expect(
      render(entry({ refresh: { added: 3, updated: 0, unavailable: 0, enabled: 3 } }), data)
    ).toContain("Found 3 models. All are enabled.");
    const failed = render(entry({ problem: "Key rejected." }), data);
    expect(failed).toContain("The connection test failed: Key rejected.");
    expect(failed).toContain("No models yet");
  });
});
