// The chat view without streaming: the chat request helpers, rename and model rules, tools and
// the overflow menu, the instructions drawer, safe Markdown, saved messages, and the screen.

import type {
  ChatResponse,
  MessageResponse,
  ModelResponse,
  ProjectDetailsResponse,
  ProjectResponse,
  UserResponse,
} from "@copilot/contracts";
import { afterEach, describe, expect, test } from "@rstest/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { resetTransport, setTransport } from "@web/api/client";
import {
  deleteChat,
  markRead,
  saveAsProject,
  saveInstructions,
  updateChat,
} from "@web/api/routes/chats";
import { keys } from "@web/api/routes/keys";
import { clearToken, setToken } from "@web/auth/token";
import { Markdown, nodeText } from "@web/components/Chat/Markdown";
import { callStatus, MessageList, resultText } from "@web/components/Chat/MessageList";
import { inlineImage, safeLink, transformUrl } from "@web/components/Chat/urls";
import { Menu } from "@web/components/Menu/Menu";
import { ModelPicker, Sheet } from "@web/components/ModelPicker/ModelPicker";
import { InstructionsForm, MoveForm } from "@web/routes/chat/dialogs";
import ChatPage from "@web/routes/chat/page";
import {
  contextUse,
  insertInstructions,
  menuActions,
  modelNotice,
  titleSchema,
  toolResults,
  toolsOn,
} from "@web/routes/chat/settings";
import path from "@web/routes/path";
import type { ReactNode } from "react";
import { renderToString } from "react-dom/server";
import { MemoryRouter, Route, Routes } from "react-router-dom";

const AT = "2026-11-26T10:00:00Z";
const CHAT = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708192";
const PROJECT = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708200";
const OTHER = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708201";
const MODEL = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708101";
const SERVER = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708300";
const CALL = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708400";
const FILE = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708210";

const chat: ChatResponse = {
  id: CHAT,
  projectId: null,
  modelId: MODEL,
  scheduleId: null,
  title: "Trip plans",
  instructions: "",
  reasoning: null,
  builtinTools: null,
  toolOverrides: [],
  approvedTools: [],
  pinned: false,
  voiceReplies: false,
  lastReadAt: AT,
  createdAt: AT,
  updatedAt: AT,
};

const project: ProjectResponse = {
  id: PROJECT,
  name: "Travel",
  icon: "✈️",
  instructions: "Plan cheap trips.",
  defaultModelId: null,
  reasoning: null,
  builtinTools: [],
  toolServerIds: [],
  pinned: false,
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

function model(id: string, displayName: string, extra: Partial<ModelResponse> = {}): ModelResponse {
  return {
    id,
    providerId: "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708100",
    providerName: "Mock",
    kind: "openai",
    preset: "custom",
    remoteId: displayName.toLowerCase(),
    displayName,
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
    ...extra,
  };
}

function message(
  id: string,
  role: MessageResponse["role"],
  parts: MessageResponse["parts"],
  extra: Partial<MessageResponse> = {}
): MessageResponse {
  return {
    id,
    chatId: CHAT,
    role,
    status: "complete",
    finishReason: null,
    modelId: null,
    parts,
    usage: null,
    error: null,
    createdAt: AT,
    updatedAt: AT,
    ...extra,
  };
}

const user: UserResponse = {
  id: "0195d6a4-1b2c-7d3e-8f40-123456789abc",
  username: "ada",
  displayName: "Ada",
  role: "user",
  disabled: false,
  createdAt: AT,
  updatedAt: AT,
};

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

/** Renders `element` with a session and query data. */
function render(element: ReactNode, data: [readonly unknown[], unknown][] = [], url = "/") {
  const client = new QueryClient();
  setToken("token");
  client.setQueryData(["auth", "me", "token"], user);
  for (const [key, value] of data) client.setQueryData(key, value);
  return renderToString(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[url]}>{element}</MemoryRouter>
    </QueryClientProvider>
  );
}

afterEach(() => {
  calls = [];
  clearToken();
  resetTransport();
});

describe("the chat request helpers", () => {
  test("change, read, delete, and save a chat", async () => {
    serve(() => json({ data: { ...chat, projectId: null } }));
    await updateChat(CHAT, { projectId: null, title: "Trips" });
    expect(calls[0]).toMatchObject({ url: `/api/chats/${CHAT}`, method: "PATCH" });
    expect(JSON.parse(String(calls[0]?.body))).toEqual({ projectId: null, title: "Trips" });

    calls = [];
    serve(() => new Response(null, { status: 204 }));
    await markRead(CHAT);
    await deleteChat(CHAT);
    expect(calls.map((call) => [call.method, call.url])).toEqual([
      ["POST", `/api/chats/${CHAT}/read`],
      ["DELETE", `/api/chats/${CHAT}`],
    ]);

    calls = [];
    serve(() => json({ data: project }));
    expect((await saveAsProject(CHAT)).id).toBe(PROJECT);
    expect((await saveInstructions(CHAT)).id).toBe(PROJECT);
    expect(calls.map((call) => [call.method, call.url])).toEqual([
      ["POST", `/api/chats/${CHAT}/save-as-project`],
      ["POST", `/api/chats/${CHAT}/save-instructions`],
    ]);
  });
});

describe("chat settings", () => {
  test("a title is one line of at most 100 characters", () => {
    expect(titleSchema.safeParse("  Plans ").data).toBe("Plans");
    expect(titleSchema.safeParse(" ").error?.issues[0]?.message).toBe("Title is required.");
    expect(titleSchema.safeParse("a\nb").error?.issues[0]?.message).toBe(
      "Title must be a single line."
    );
    expect(titleSchema.safeParse("x".repeat(101)).error?.issues[0]?.message).toBe(
      "Title must be at most 100 characters."
    );
  });

  test("asks for a model when the chat has none or its model is gone", () => {
    expect(modelNotice({ modelId: null }, undefined)).toBe("Choose a model to continue.");
    expect(modelNotice({ modelId: MODEL }, [model(OTHER, "Other")])).toContain(
      "turned off or no longer listed"
    );
    expect(modelNotice({ modelId: MODEL }, [model(MODEL, "GPT")])).toBeNull();
    expect(modelNotice({ modelId: MODEL }, undefined)).toBeNull();
  });

  test("tools are on with the project's servers after the chat's overrides, or built-in tools", () => {
    const withServer = { ...project, toolServerIds: [SERVER] };
    expect(toolsOn(chat, null)).toBe(false);
    expect(toolsOn(chat, withServer)).toBe(true);
    expect(
      toolsOn({ ...chat, toolOverrides: [{ serverId: SERVER, enabled: false }] }, withServer)
    ).toBe(false);
    expect(toolsOn({ ...chat, toolOverrides: [{ serverId: SERVER, enabled: true }] }, null)).toBe(
      true
    );
    const schedules = { ...project, builtinTools: ["schedules" as const] };
    expect(toolsOn(chat, schedules)).toBe(true);
    expect(toolsOn({ ...chat, builtinTools: [] }, schedules)).toBe(false);
  });

  test("the menu offers save as project alone, and save instructions in a project", () => {
    expect(menuActions(chat)).toEqual(["instructions", "move", "pin", "save-as-project", "delete"]);
    expect(menuActions({ projectId: PROJECT, instructions: "Be brief." })).toEqual([
      "project",
      "instructions",
      "move",
      "pin",
      "save-instructions",
      "delete",
    ]);
    expect(menuActions({ projectId: PROJECT, instructions: " " })).not.toContain(
      "save-instructions"
    );
  });

  test("inserting another project's instructions appends them after a blank line", () => {
    expect(insertInstructions("", " Be brief. ")).toBe("Be brief.");
    expect(insertInstructions("Use metric units.\n", "Be brief.")).toBe(
      "Use metric units.\n\nBe brief."
    );
    expect(insertInstructions("Keep me", "  ")).toBe("Keep me");
  });
});

describe("the context meter", () => {
  const usage = (inputTokens: number | null) => ({
    inputTokens,
    outputTokens: 10,
    reasoningTokens: null,
    cacheReadTokens: null,
    cacheWriteTokens: null,
  });

  test("reads the last answer's input tokens against the window", () => {
    const messages = [
      message("0196f6a4-7c1e-7d2a-9b3c-4d5e6f708601", "assistant", [], { usage: usage(1_000) }),
      message("0196f6a4-7c1e-7d2a-9b3c-4d5e6f708602", "assistant", [], { usage: usage(32_000) }),
      message("0196f6a4-7c1e-7d2a-9b3c-4d5e6f708603", "assistant", [], { usage: usage(null) }),
      message("0196f6a4-7c1e-7d2a-9b3c-4d5e6f708604", "user", []),
    ];
    expect(contextUse(messages, { contextWindow: 128_000 })).toEqual({
      used: 32_000,
      window: 128_000,
      percent: 25,
    });
    expect(contextUse(messages, { contextWindow: null })).toBeNull();
    expect(contextUse(messages, undefined)).toBeNull();
    expect(contextUse([], { contextWindow: 128_000 })).toBeNull();
  });
});

describe("safe output", () => {
  test("links keep only http, https, and mailto", () => {
    expect(safeLink("https://example.com/a?b=1")).toBe("https://example.com/a?b=1");
    expect(safeLink("mailto:ada@example.com")).toBe("mailto:ada@example.com");
    for (const bad of [
      "javascript:alert(1)",
      "data:text/html,x",
      "/settings",
      "ftp://x",
      "vbscript:x",
    ]) {
      expect(safeLink(bad)).toBeNull();
    }
  });

  test("only blob and data images render inline", () => {
    expect(inlineImage("blob:https://app/1")).toBe(true);
    expect(inlineImage("data:image/png;base64,AAAA")).toBe(true);
    expect(inlineImage("data:text/html,x")).toBe(false);
    expect(inlineImage("https://example.com/a.png")).toBe(false);
    expect(transformUrl("https://example.com/a.png", "src")).toBe("https://example.com/a.png");
    expect(transformUrl("javascript:alert(1)", "href")).toBe("");
    expect(transformUrl("data:image/png;base64,AAAA", "href")).toBe("");
  });

  test("Markdown never renders raw HTML or unsafe links", () => {
    const html = renderToString(
      <Markdown>
        {
          "<img src=x onerror=alert(1)> **bold** [click](javascript:alert(1)) [docs](https://example.com/docs) ![pic](https://example.com/p.png) ![dot](data:image/png;base64,AAAA)"
        }
      </Markdown>
    );
    expect(html).not.toContain('<img src="x"');
    expect(html).toContain("&lt;img src=x onerror=alert(1)&gt;");
    expect(html).toContain("<strong>bold</strong>");
    expect(html).not.toContain("javascript:");
    expect(html).toContain('href="https://example.com/docs"');
    expect(html).toContain('rel="noopener noreferrer"');
    expect(html).toContain("(https://example.com/docs)");
    expect(html).toContain(">https://example.com/p.png</a>");
    expect(html).toContain('<img src="data:image/png;base64,AAAA"');
  });

  test("a link whose text is its URL is not repeated, and code blocks can be copied", () => {
    const html = renderToString(
      <Markdown>{"<https://example.com>\n\n```js\nconst a = 1;\n```"}</Markdown>
    );
    expect(html).not.toContain("(https://example.com/)");
    expect(html).toContain('aria-label="Copy code"');
    expect(html).toContain("const a = 1;");
    expect(
      nodeText({
        type: "element",
        children: [
          { type: "text", value: "a" },
          { type: "text", value: "b" },
        ],
      })
    ).toBe("ab");
  });
});

describe("saved messages", () => {
  const call = {
    type: "toolCall" as const,
    callId: CALL,
    serverId: SERVER,
    toolName: "search_web",
    arguments: { query: "trains" },
    complete: true,
    approval: "auto" as const,
    denyReason: null,
  };
  const result = {
    type: "toolResult" as const,
    callId: CALL,
    status: "ok" as const,
    isError: false,
    content: [
      { type: "text" as const, text: "3 results" },
      { type: "image" as const, fileId: FILE },
    ],
    truncated: true,
  };

  test("tool calls read their result's status, else their approval", () => {
    expect(callStatus(call, result).label).toBe("Done");
    expect(callStatus({ ...call, approval: "pending" }, undefined).label).toBe("Needs approval");
    expect(callStatus({ ...call, approval: "denied" }, undefined).label).toBe("Denied");
    expect(callStatus(call, { ...result, status: "timeout" }).label).toBe("Timed out");
    expect(resultText(result)).toBe("3 results\n[image]");
    const messages = [
      message("0196f6a4-7c1e-7d2a-9b3c-4d5e6f708501", "assistant", [call]),
      message("0196f6a4-7c1e-7d2a-9b3c-4d5e6f708502", "tool", [result]),
    ];
    expect(toolResults(messages).get(CALL)).toEqual(result);
  });

  test("show reasoning folded, Markdown answers, tool calls, files, and stops", () => {
    const html = renderToString(
      <QueryClientProvider client={new QueryClient()}>
        <MessageList
          messages={[
            message("0196f6a4-7c1e-7d2a-9b3c-4d5e6f708500", "user", [
              { type: "file", fileId: FILE, name: "ticket.pdf", mediaType: "application/pdf" },
              { type: "text", text: "Find **trains**" },
            ]),
            message("0196f6a4-7c1e-7d2a-9b3c-4d5e6f708501", "assistant", [
              { type: "reasoning", text: "Thinking about trains" },
              call,
            ]),
            message("0196f6a4-7c1e-7d2a-9b3c-4d5e6f708502", "tool", [result]),
            message(
              "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708503",
              "assistant",
              [{ type: "text", text: "Take the **8:15**." }],
              { status: "cancelled" }
            ),
          ]}
        />
      </QueryClientProvider>
    );
    expect(html).toContain('id="message-0196f6a4-7c1e-7d2a-9b3c-4d5e6f708500"');
    expect(html).toContain("ticket.pdf");
    // The user's own text is shown as typed.
    expect(html).toContain("Find **trains**");
    expect(html).toContain("<summary");
    expect(html).toContain("Reasoning");
    expect(html).toContain("search_web");
    expect(html).toContain("Done");
    expect(html).toContain("&quot;query&quot;: &quot;trains&quot;");
    expect(html).toContain("The result was cut to the size limit.");
    expect(html).toContain("<strong>8:15</strong>");
    expect(html).toContain("Stopped.");
    expect(html).not.toContain('id="message-0196f6a4-7c1e-7d2a-9b3c-4d5e6f708502"');
  });
});

describe("the chat view", () => {
  test("the menu starts closed", () => {
    const html = renderToString(
      <Menu label="Chat actions" items={[{ key: "a", label: "Pin", onSelect: () => undefined }]} />
    );
    expect(html).toContain('aria-label="Chat actions"');
    expect(html).toContain('aria-expanded="false"');
    expect(html).not.toContain("Pin");
  });

  test("the model picker names the model, its level, and the models by connection", () => {
    const thinking = model(MODEL, "Thinker", {
      capabilities: { ...capabilities, reasoning: true, reasoningLevels: ["low", "high"] },
      reasoningOffWithTools: true,
    });
    const other = model(OTHER, "Other", { providerId: SERVER, providerName: "Second" });
    const button = render(
      <ModelPicker
        models={[thinking, other]}
        modelId={MODEL}
        reasoning="high"
        onChange={() => undefined}
      />
    );
    expect(button).toContain("Model: </span>");
    expect(button).toContain("Thinker");
    expect(button).toContain(">High<");

    const sheet = render(
      <Sheet
        models={[thinking, other]}
        initial={{ modelId: MODEL, reasoning: "high" }}
        toolsOn
        onCancel={() => undefined}
        onPick={async () => undefined}
      />
    );
    expect(sheet).toContain("<legend");
    expect(sheet).toContain(">Mock</legend>");
    expect(sheet).toContain(">Second</legend>");
    expect(sheet).toContain("Model default");
    expect(sheet).toContain("turns reasoning off while tools are on");
    expect(
      render(<ModelPicker models={[]} modelId={null} reasoning={null} onChange={() => undefined} />)
    ).toContain("No models");
    expect(
      render(
        <Sheet
          models={[]}
          initial={{ modelId: "", reasoning: null }}
          toolsOn={false}
          onCancel={() => undefined}
          onPick={async () => undefined}
        />
      )
    ).toContain("Add a provider");
  });

  test("the instructions drawer shows the project's, offers others, and saves to the project", () => {
    const html = render(
      <InstructionsForm
        chat={{ ...chat, projectId: PROJECT, instructions: "Use trains." }}
        project={project}
        onDone={() => undefined}
      />,
      [
        [
          keys.projectList({}),
          [project, { ...project, id: OTHER, name: "Style", icon: "", instructions: "Be brief." }],
        ],
      ]
    );
    expect(html).toContain("From Travel");
    expect(html).toContain("Plan cheap trips.");
    expect(html).toContain("Use trains.");
    expect(html).toContain("Insert from project...");
    expect(html).toContain(`<option value="${OTHER}">Style</option>`);
    expect(html).not.toContain(`<option value="${PROJECT}">`);
    expect(html).toContain("Save to project");
  });

  test("moving offers no project and every project, with the current one chosen", () => {
    const html = render(
      <MoveForm chat={{ ...chat, projectId: PROJECT }} onDone={() => undefined} />,
      [[keys.projectList({}), [project]]]
    );
    expect(html).toContain("No project");
    expect(html).toContain("✈️ Travel");
    expect(html).toContain(`checked="" value="${PROJECT}"`);
  });

  test("a chat shows its title, model, project, and actions", () => {
    const details: ProjectDetailsResponse = {
      ...project,
      capacity: { used: 0, window: null, level: "unknown" },
    };
    const html = render(
      <Routes>
        <Route path={path.chat} element={<ChatPage />} />
      </Routes>,
      [
        [keys.chat(CHAT), { ...chat, projectId: PROJECT }],
        [keys.chatMessages(CHAT), []],
        [keys.models, [model(MODEL, "GPT")]],
        [keys.project(PROJECT), details],
      ],
      `/chats/${CHAT}`
    );
    expect(html).toContain('title="Rename"');
    expect(html).toContain("Trip plans");
    expect(html).toContain(">GPT</span>");
    expect(html).toContain('aria-label="Chat actions"');
    expect(html).toContain(`href="/projects/${PROJECT}"`);
    expect(html).toContain("✈️ Travel");
    expect(html).not.toContain("Choose a model to continue.");
  });

  test("a chat without a model asks for one and cannot send", () => {
    const html = render(
      <Routes>
        <Route path={path.chat} element={<ChatPage />} />
      </Routes>,
      [
        [keys.chat(CHAT), { ...chat, modelId: null }],
        [keys.chatMessages(CHAT), []],
        [keys.models, [model(MODEL, "GPT")]],
      ],
      `/chats/${CHAT}`
    );
    expect(html).toContain("Choose a model to continue.");
    expect(html).toContain("Choose a model</span>");
    expect(html).toMatch(/<textarea[^>]*disabled=""/);
  });
});
