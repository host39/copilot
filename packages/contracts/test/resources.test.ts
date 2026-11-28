// Parses sample payloads of the settings, provider, model, file, project, tool server, and
// schedule routes with the Zod schemas. The response samples have the shapes the Rust tests in
// `packages/contracts/src/` pin, with timestamps as chrono writes them.

import {
  createProviderRequestSchema,
  createToolServerRequestSchema,
  fileBodySchema,
  modelListBodySchema,
  projectDetailsBodySchema,
  providerDetailsBodySchema,
  scheduleBodySchema,
  settingsBodySchema,
  toolServerBodySchema,
  updateModelRequestSchema,
  updateProjectRequestSchema,
  updateProviderRequestSchema,
  updateToolServerRequestSchema,
} from "@contracts/index";
import { describe, expect, test } from "@rstest/core";

const ID = "019abf6c-2f6e-7a51-8c3e-6d1e2f3a4b5c";
const AT = "2026-11-26T12:00:00Z";
const AT_NANOS = "2026-11-26T12:00:00.123456789Z";

const capabilities = {
  vision: true,
  pdf: false,
  tools: true,
  audioIn: false,
  audioOut: false,
  reasoning: true,
  reasoningLevels: ["low", "medium", "high"],
  defaultReasoning: null,
};

describe("responses", () => {
  test("settings", () => {
    expect(settingsBodySchema.parse({ data: { instructions: "" } }).data.instructions).toBe("");
  });

  test("provider details carry usage beside the provider fields", () => {
    const { data } = providerDetailsBodySchema.parse({
      data: {
        id: ID,
        kind: "openai",
        preset: "openrouter",
        api: "chat_completions",
        name: "OpenRouter",
        baseUrl: "https://openrouter.ai/api/v1",
        enabled: true,
        key: "unreadable",
        createdAt: AT,
        updatedAt: AT_NANOS,
        usage: { chats: 3, projects: 1, schedules: 0 },
      },
    });
    expect(data.key).toBe("unreadable");
    expect(data.api).toBe("chat_completions");
    expect(data.usage.chats).toBe(3);
  });

  test("an OpenAI connection names its API; an Anthropic one has none", () => {
    expect(
      createProviderRequestSchema.parse({
        kind: "openai",
        preset: "custom",
        api: "responses",
        name: "G",
      }).api
    ).toBe("responses");
    expect(() =>
      createProviderRequestSchema.parse({
        kind: "openai",
        preset: "openai",
        api: "completions",
        name: "O",
      })
    ).toThrow();
    expect(updateProviderRequestSchema.parse({ api: "chat_completions" }).api).toBe(
      "chat_completions"
    );
  });

  test("models with locks and overrides", () => {
    const body = modelListBodySchema.parse({
      items: [
        {
          id: ID,
          providerId: ID,
          providerName: "Anthropic",
          kind: "anthropic",
          preset: "anthropic",
          remoteId: "claude-model",
          displayName: "Claude",
          enabled: true,
          available: true,
          contextWindow: 200000,
          maxOutputTokens: null,
          capabilities,
          detected: capabilities,
          overrides: {
            vision: null,
            pdf: true,
            tools: null,
            audioIn: null,
            audioOut: null,
            reasoning: null,
          },
          locks: [{ capability: "audioIn", reason: "Claude models do not take or return audio." }],
          reasoningOffWithTools: false,
          createdAt: AT,
          updatedAt: AT,
        },
      ],
      total: 1,
    });
    expect(body.items[0]?.locks[0]?.capability).toBe("audioIn");
  });

  test("file with one owner", () => {
    const { data } = fileBodySchema.parse({
      data: {
        id: ID,
        projectId: null,
        chatId: ID,
        name: "notes.md",
        mediaType: "text/markdown",
        sizeBytes: 1200,
        tokenEstimate: 300,
        pageCount: null,
        included: true,
        createdAt: AT,
      },
    });
    expect(data.chatId).toBe(ID);
  });

  test("project details carry the capacity bar", () => {
    const { data } = projectDetailsBodySchema.parse({
      data: {
        id: ID,
        name: "Research",
        icon: "",
        instructions: "",
        defaultModelId: null,
        reasoning: null,
        builtinTools: ["schedules"],
        toolServerIds: [ID],
        pinned: false,
        createdAt: AT,
        updatedAt: AT,
        capacity: { used: 9000, window: 10000, level: "warn" },
      },
    });
    expect(data.capacity.level).toBe("warn");
  });

  test("tool server before its first test", () => {
    const { data } = toolServerBodySchema.parse({
      data: {
        id: ID,
        name: "Docs",
        slug: "docs",
        transport: "http",
        url: "https://mcp.example.com/mcp",
        stdio: null,
        approval: "ask",
        toolModes: { search_docs: "off" },
        enabled: true,
        credentials: "missing",
        authMode: "oauth",
        oauth: {
          state: "needs_sign_in",
          clientId: null,
          clientSecret: "missing",
          expiresAt: null,
          redirectUri: null,
        },
        protocolVersion: null,
        tools: null,
        toolsFetchedAt: null,
        reviewRequired: false,
        createdAt: AT,
        updatedAt: AT,
      },
    });
    expect(data.toolModes.search_docs).toBe("off");
    expect(data.oauth?.state).toBe("needs_sign_in");
  });

  test("stdio tool server: the program without its environment's values", () => {
    const { data } = toolServerBodySchema.parse({
      data: {
        id: ID,
        name: "Files",
        slug: "files",
        transport: "stdio",
        url: null,
        stdio: {
          command: "/usr/local/bin/files-mcp",
          args: ["--root", "/srv/data dir"],
          envNames: ["API_KEY"],
          env: "set",
          cwd: ID,
        },
        approval: "ask",
        toolModes: {},
        enabled: true,
        credentials: "missing",
        authMode: "none",
        oauth: null,
        protocolVersion: null,
        tools: null,
        toolsFetchedAt: null,
        reviewRequired: false,
        createdAt: AT,
        updatedAt: AT,
      },
    });
    expect(data.url).toBeNull();
    expect(data.stdio?.args).toEqual(["--root", "/srv/data dir"]);
    const request = createToolServerRequestSchema.parse({
      name: "Files",
      transport: "stdio",
      stdio: { command: "/usr/local/bin/files-mcp" },
    });
    expect(request.stdio?.args).toEqual([]);
    expect(request.stdio?.env).toEqual({});
  });

  test("schedule paused by the scheduler", () => {
    const { data } = scheduleBodySchema.parse({
      data: {
        id: ID,
        projectId: null,
        modelId: null,
        sourceChatId: null,
        name: "Morning news",
        cron: "0 7 * * 1-5",
        timezone: "Europe/Berlin",
        description: "Weekdays at 07:00",
        prompt: "Summarize the news.",
        instructions: "",
        reasoning: null,
        onAsk: "deny",
        createdBy: "assistant",
        enabled: false,
        nextRunAt: null,
        lastRunAt: AT,
        lastStatus: "awaiting_approval",
        lastError: null,
        consecutiveFailures: 0,
        disabledReason: "Model removed",
        createdAt: AT,
        updatedAt: AT,
      },
    });
    expect(data.lastStatus).toBe("awaiting_approval");
  });

  test("rejects values the server never sends", () => {
    const result = settingsBodySchema.safeParse({ data: {} });
    expect(result.success).toBe(false);
    const status = scheduleBodySchema.shape.data.shape.lastStatus.safeParse("awaitingApproval");
    expect(status.success).toBe(false);
  });
});

describe("requests", () => {
  test("PATCH keeps null apart from an absent field", () => {
    expect(updateProjectRequestSchema.parse({ defaultModelId: null })).toEqual({
      defaultModelId: null,
    });
    expect(updateProjectRequestSchema.parse({})).toEqual({});
    expect(updateModelRequestSchema.parse({ contextWindow: null }).contextWindow).toBeNull();
    expect(updateToolServerRequestSchema.parse({ credentials: null }).credentials).toBeNull();
  });

  test("tool server headers default to none", () => {
    const request = createToolServerRequestSchema.parse({
      name: "Docs",
      url: "https://mcp.example.com/mcp",
    });
    expect(request.headers).toEqual({});
  });
});
