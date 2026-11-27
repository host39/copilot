// The request helpers of the Settings screens and the first-run setup: each one's method, path,
// query, and body, and how it reads the answer.

import { afterEach, describe, expect, test } from "@rstest/core";
import { resetTransport, setTransport } from "@web/api/client";
import {
  changePassword,
  fetchSessions,
  revokeOtherSessions,
  revokeSession,
  updateProfile,
} from "@web/api/routes/auth";
import { updateModel } from "@web/api/routes/models";
import {
  createProvider,
  deleteProvider,
  fetchProvider,
  fetchProviderModels,
  fetchProviders,
  refreshProvider,
  updateProvider,
} from "@web/api/routes/providers";
import { fetchSettings, updateSettings } from "@web/api/routes/settings";
import {
  createToolServer,
  deleteToolServer,
  fetchToolServer,
  fetchToolServers,
  reviewToolServer,
  testToolServer,
  updateToolServer,
} from "@web/api/routes/tool-servers";
import {
  createUser,
  deleteUser,
  fetchUser,
  fetchUsers,
  unlockUser,
  updateUser,
} from "@web/api/routes/users";

type Call = { url: string; method: string; body: BodyInit | null | undefined };

let calls: Call[] = [];

function serve(answer: () => Response): void {
  setTransport({
    server: () => "",
    token: () => "token",
    fetch: async (input, init) => {
      calls.push({ url: input, method: init?.method ?? "GET", body: init?.body });
      return answer();
    },
  });
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

const empty = () => new Response(null, { status: 204 });

/** The one request the last call sent. */
function sent(): { url: string; method: string; body: unknown } {
  expect(calls).toHaveLength(1);
  const call = calls[0] as Call;
  return {
    url: call.url,
    method: call.method,
    body: typeof call.body === "string" ? JSON.parse(call.body) : call.body,
  };
}

const AT = "2026-11-26T10:00:00Z";
const ID = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708100";
const MODEL = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708101";

const user = {
  id: ID,
  username: "grace",
  displayName: "Grace",
  role: "user",
  disabled: false,
  createdAt: AT,
  updatedAt: AT,
};

const session = {
  id: ID,
  current: true,
  methods: ["password"],
  assurance: "single",
  createdAt: AT,
  lastUsedAt: AT,
  expiresAt: AT,
  userAgent: "Mozilla/5.0",
};

const provider = {
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

const model = {
  id: MODEL,
  providerId: ID,
  providerName: "OpenAI",
  kind: "openai",
  preset: "openai",
  remoteId: "gpt",
  displayName: "GPT",
  enabled: true,
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

const server = {
  id: ID,
  name: "Docs",
  slug: "docs",
  url: "https://mcp.example.com/mcp",
  approval: "ask",
  toolModes: {},
  enabled: true,
  credentials: "missing",
  protocolVersion: null,
  tools: null,
  toolsFetchedAt: null,
  reviewRequired: false,
  createdAt: AT,
  updatedAt: AT,
};

afterEach(() => {
  calls = [];
  resetTransport();
});

describe("account", () => {
  test("renames the caller", async () => {
    serve(() => json({ data: { ...user, displayName: "Grace H." } }));
    expect((await updateProfile({ displayName: "Grace H." })).displayName).toBe("Grace H.");
    expect(sent()).toEqual({
      url: "/api/auth/me",
      method: "PATCH",
      body: { displayName: "Grace H." },
    });
  });

  test("changes the password with the current one", async () => {
    serve(empty);
    await changePassword({ current: "old password!", new: "new password!!" });
    expect(sent()).toEqual({
      url: "/api/auth/password",
      method: "POST",
      body: { current: "old password!", new: "new password!!" },
    });
  });

  test("lists, revokes, and revokes the other sessions", async () => {
    serve(() => json({ items: [session], total: 1 }));
    expect((await fetchSessions())[0]?.current).toBe(true);
    expect(sent().url).toBe("/api/auth/sessions");

    calls = [];
    serve(empty);
    await revokeSession("a/b");
    expect(sent()).toMatchObject({ url: "/api/auth/sessions/a%2Fb", method: "DELETE" });

    calls = [];
    await revokeOtherSessions();
    expect(sent()).toMatchObject({ url: "/api/auth/sessions/revoke-others", method: "POST" });
  });

  test("reads and saves the personal instructions", async () => {
    serve(() => json({ data: { instructions: "Be brief." } }));
    expect(await fetchSettings()).toEqual({ instructions: "Be brief." });
    calls = [];
    await updateSettings({ instructions: "Be brief." });
    expect(sent()).toEqual({
      url: "/api/settings",
      method: "PATCH",
      body: { instructions: "Be brief." },
    });
  });
});

describe("providers", () => {
  test("lists connections and reads one with its usage", async () => {
    serve(() => json({ items: [provider], total: 1 }));
    expect((await fetchProviders())[0]?.name).toBe("OpenAI");
    expect(sent().url).toBe("/api/providers");

    calls = [];
    serve(() => json({ data: { ...provider, usage: { chats: 2, projects: 1, schedules: 0 } } }));
    expect((await fetchProvider(ID)).usage.chats).toBe(2);
    expect(sent().url).toBe(`/api/providers/${ID}`);
  });

  test("creates, updates, refreshes, and deletes a connection", async () => {
    serve(() => json({ data: provider }, 201));
    await createProvider({ kind: "openai", preset: "openai", name: "OpenAI", apiKey: "sk-1" });
    expect(sent()).toEqual({
      url: "/api/providers",
      method: "POST",
      body: { kind: "openai", preset: "openai", name: "OpenAI", apiKey: "sk-1" },
    });

    calls = [];
    serve(() => json({ data: provider }));
    await updateProvider(ID, { name: "Work" });
    expect(sent()).toEqual({
      url: `/api/providers/${ID}`,
      method: "PATCH",
      body: { name: "Work" },
    });

    calls = [];
    serve(() => json({ data: { added: 3, updated: 0, unavailable: 0, enabled: 3 } }));
    expect((await refreshProvider(ID)).added).toBe(3);
    expect(sent()).toMatchObject({ url: `/api/providers/${ID}/refresh`, method: "POST" });

    calls = [];
    serve(empty);
    await deleteProvider(ID);
    expect(sent()).toMatchObject({ url: `/api/providers/${ID}`, method: "DELETE" });
  });

  test("lists a connection's models and changes one", async () => {
    serve(() => json({ items: [model], total: 1 }));
    expect((await fetchProviderModels(ID))[0]?.remoteId).toBe("gpt");
    expect(sent().url).toBe(`/api/providers/${ID}/models`);

    calls = [];
    serve(() => json({ data: { ...model, contextWindow: null } }));
    await updateModel(MODEL, { enabled: false, contextWindow: null });
    expect(sent()).toEqual({
      url: `/api/models/${MODEL}`,
      method: "PATCH",
      body: { enabled: false, contextWindow: null },
    });
  });
});

describe("tool servers", () => {
  test("lists, reads, creates, and updates servers", async () => {
    serve(() => json({ items: [server], total: 1 }));
    expect((await fetchToolServers())[0]?.slug).toBe("docs");
    expect(sent().url).toBe("/api/tool-servers");

    calls = [];
    serve(() => json({ data: server }));
    expect((await fetchToolServer(ID)).name).toBe("Docs");
    expect(sent().url).toBe(`/api/tool-servers/${ID}`);

    calls = [];
    serve(() => json({ data: server }, 201));
    await createToolServer({
      name: "Docs",
      url: "https://mcp.example.com/mcp",
      headers: { "X-Team": "a" },
      approval: "ask",
    });
    expect(sent()).toMatchObject({
      url: "/api/tool-servers",
      method: "POST",
      body: { name: "Docs", headers: { "X-Team": "a" }, approval: "ask" },
    });

    calls = [];
    serve(() => json({ data: server }));
    await updateToolServer(ID, { credentials: null, toolModes: { search: "auto" } });
    expect(sent()).toEqual({
      url: `/api/tool-servers/${ID}`,
      method: "PATCH",
      body: { credentials: null, toolModes: { search: "auto" } },
    });
  });

  test("tests, reviews, and deletes a server", async () => {
    serve(() =>
      json({ data: { server, changes: { added: ["search"], removed: [], changed: [] } } })
    );
    expect((await testToolServer(ID)).changes.added).toEqual(["search"]);
    expect(sent()).toMatchObject({ url: `/api/tool-servers/${ID}/test`, method: "POST" });

    calls = [];
    serve(() => json({ data: server }));
    await reviewToolServer(ID);
    expect(sent()).toMatchObject({ url: `/api/tool-servers/${ID}/review`, method: "POST" });

    calls = [];
    serve(empty);
    await deleteToolServer(ID);
    expect(sent()).toMatchObject({ url: `/api/tool-servers/${ID}`, method: "DELETE" });
  });
});

describe("users", () => {
  test("pages through accounts 100 at a time", async () => {
    serve(() => json({ items: [user], total: 101 }));
    const page = await fetchUsers(100);
    expect(page.total).toBe(101);
    expect(sent().url).toBe("/api/admin/users?offset=100&limit=100");
  });

  test("an unknown role from a newer server reads as user", async () => {
    serve(() => json({ data: { ...user, role: "auditor" } }));
    expect((await fetchUser(ID)).role).toBe("user");
  });

  test("creates, updates, unlocks, and deletes accounts", async () => {
    serve(() => json({ data: user }, 201));
    await createUser({
      username: "grace",
      displayName: "Grace",
      temporaryPassword: "temporary password",
      role: "user",
    });
    expect(sent()).toMatchObject({ url: "/api/admin/users", method: "POST" });

    calls = [];
    serve(() => json({ data: { ...user, disabled: true } }));
    expect((await updateUser(ID, { disabled: true })).disabled).toBe(true);
    expect(sent()).toEqual({
      url: `/api/admin/users/${ID}`,
      method: "PATCH",
      body: { disabled: true },
    });

    calls = [];
    serve(empty);
    await unlockUser(ID);
    expect(sent()).toMatchObject({ url: `/api/admin/users/${ID}/unlock`, method: "POST" });

    calls = [];
    await deleteUser(ID);
    expect(sent()).toMatchObject({ url: `/api/admin/users/${ID}`, method: "DELETE" });
  });
});
