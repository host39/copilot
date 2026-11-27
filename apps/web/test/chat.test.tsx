// The walking skeleton's chat: the streamed reply, the chat API helpers, and the two screens.

import type {
  ChatStreamEvent,
  MessageResponse,
  ModelResponse,
  UserResponse,
} from "@copilot/contracts";
import { afterEach, describe, expect, test } from "@rstest/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { resetTransport, setTransport } from "@web/api/client";
import { createChat, fetchMessages, sendMessage } from "@web/api/routes/chats";
import { fetchModels } from "@web/api/routes/models";
import { clearToken, setToken } from "@web/auth/token";
import { MessageList, messageText } from "@web/components/Chat/MessageList";
import ChatPage from "@web/routes/chat/page";
import { apply, emptyRun, view } from "@web/routes/chat/stream";
import HomePage from "@web/routes/home/page";
import path from "@web/routes/path";
import { renderToString } from "react-dom/server";
import { MemoryRouter, Route, Routes } from "react-router-dom";

const CHAT = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708192";
const REPLY = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708193";
const MODEL = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708101";
const AT = "2026-11-26T10:00:00Z";

type Call = { url: string; method: string; headers: Headers; body: BodyInit | null | undefined };
let calls: Call[] = [];

function serve(answer: (call: Call) => Response): void {
  setToken("token");
  setTransport({
    server: () => "",
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

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function events(values: ChatStreamEvent[]): Response {
  const body = values.map((value) => `data: ${JSON.stringify(value)}\n\n`).join("");
  return new Response(body, { headers: { "Content-Type": "text/event-stream" } });
}

/** A turn that answers "Hello!" in two deltas. */
const turn: ChatStreamEvent[] = [
  { type: "message.started", seq: 1, messageId: REPLY, role: "assistant", windowStartId: null },
  { type: "part.started", seq: 2, messageId: REPLY, index: 0, kind: "text" },
  { type: "text.delta", seq: 3, messageId: REPLY, index: 0, text: "Hel" },
  { type: "text.delta", seq: 4, messageId: REPLY, index: 0, text: "lo!" },
  {
    type: "message.finished",
    seq: 5,
    messageId: REPLY,
    status: "complete",
    finishReason: "stop",
    refusal: null,
    usage: null,
  },
  { type: "done", seq: 6 },
];

function message(
  id: string,
  role: MessageResponse["role"],
  text: string,
  extra: Partial<MessageResponse> = {}
): MessageResponse {
  return {
    id,
    chatId: CHAT,
    role,
    status: "complete",
    finishReason: role === "assistant" ? "stop" : null,
    modelId: null,
    parts: [{ type: "text", text }],
    usage: null,
    error: null,
    createdAt: AT,
    updatedAt: AT,
    ...extra,
  };
}

function model(id: string, displayName: string): ModelResponse {
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
  };
}

const admin: UserResponse = {
  id: "0195d6a4-1b2c-7d3e-8f40-123456789abc",
  username: "ada",
  displayName: "Ada",
  role: "administrator",
  disabled: false,
  createdAt: AT,
  updatedAt: AT,
};

/** Renders one screen at `url` with query data already loaded. */
function render(url: string, data: [readonly unknown[], unknown][]): string {
  const client = new QueryClient();
  setToken("token");
  client.setQueryData(["auth", "me", "token"], admin);
  for (const [key, value] of data) client.setQueryData(key, value);
  return renderToString(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[url]}>
        <Routes>
          <Route path={path.home} element={<HomePage />} />
          <Route path={path.chat} element={<ChatPage />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>
  );
}

afterEach(() => {
  calls = [];
  clearToken();
  resetTransport();
});

describe("the chat API", () => {
  test("lists the picker's models", async () => {
    serve(() => json({ items: [model(MODEL, "GPT")], total: 1 }));
    const models = await fetchModels();
    expect(models.map((item) => item.displayName)).toEqual(["GPT"]);
    expect(calls[0]?.url).toBe("/api/models");
  });

  test("creates a chat with its model", async () => {
    serve(() =>
      json(
        {
          data: {
            id: CHAT,
            projectId: null,
            modelId: MODEL,
            scheduleId: null,
            title: "",
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
          },
        },
        201
      )
    );
    const chat = await createChat({ modelId: MODEL });
    expect(chat.id).toBe(CHAT);
    expect(calls[0]?.method).toBe("POST");
    expect(calls[0]?.body).toBe(JSON.stringify({ modelId: MODEL }));
  });

  test("streams a turn's events in order", async () => {
    serve(() => events(turn));
    const seen: ChatStreamEvent[] = [];
    await sendMessage(CHAT, { text: "Say hello", fileIds: [] }, (event) => seen.push(event));
    expect(seen.map((event) => event.seq)).toEqual([1, 2, 3, 4, 5, 6]);
    expect(calls[0]?.url).toBe(`/api/chats/${CHAT}/messages`);
    expect(calls[0]?.headers.get("Accept")).toBe("text/event-stream");
    expect(calls[0]?.body).toBe('{"text":"Say hello","fileIds":[]}');
  });

  test("reads the saved messages", async () => {
    serve(() =>
      json({ items: [message("0196f6a4-7c1e-7d2a-9b3c-4d5e6f708111", "user", "Hi")], total: 1 })
    );
    const saved = await fetchMessages(CHAT);
    expect(saved[0]?.parts).toEqual([{ type: "text", text: "Hi" }]);
  });
});

describe("the chat screens", () => {
  test("model output is shown as text, never as HTML", () => {
    const live = view(
      [
        turn[0],
        { type: "text.delta", seq: 2, messageId: REPLY, index: 0, text: "<b>live</b>" },
      ].reduce<ReturnType<typeof apply>>(
        (run, event) => apply(run, event as ChatStreamEvent),
        emptyRun
      )
    );
    const html = renderToString(
      <MessageList
        messages={[
          message(
            "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708112",
            "assistant",
            "<img src=x onerror=alert(1)>"
          ),
          ...live,
        ]}
        writing={new Set([REPLY])}
      />
    );
    expect(html.replaceAll("<!-- -->", "")).toContain("&lt;b&gt;live&lt;/b&gt;");
    expect(html).not.toContain("<img");
    expect(html).not.toContain("<b>");
    expect(html).toContain("&lt;img");
  });

  test("a message's text joins its text parts", () => {
    expect(
      messageText([
        { type: "text", text: "One" },
        { type: "reasoning", text: "hidden" },
        { type: "text", text: "Two" },
      ])
    ).toBe("One\n\nTwo");
  });

  test("the new chat screen starts with the first enabled model and no project", () => {
    const html = render(path.home, [
      [["models"], [model(MODEL, "GPT"), model("0196f6a4-7c1e-7d2a-9b3c-4d5e6f708102", "Other")]],
    ]);
    expect(html).toContain("New Chat");
    expect(html).toContain("Model: </span>");
    expect(html).toContain(">GPT</span>");
    expect(html).toContain("No project");
  });

  test("without models it points to the first-run setup and the provider settings", () => {
    const html = render(path.home, [[["models"], []]]);
    expect(html).toContain("No models yet");
    expect(html).toContain(`href="${path.welcome}"`);
    expect(html).toContain("Add a provider");
    expect(html).toContain(`href="${path.providers}"`);
  });

  test("a chat shows its title, saved messages, and errors", () => {
    const html = render(`/chats/${CHAT}`, [
      [
        ["chats", CHAT],
        {
          id: CHAT,
          projectId: null,
          modelId: MODEL,
          scheduleId: null,
          title: "Say hello",
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
        },
      ],
      [
        ["chats", CHAT, "messages"],
        [
          message("0196f6a4-7c1e-7d2a-9b3c-4d5e6f708111", "user", "Say hello"),
          message("0196f6a4-7c1e-7d2a-9b3c-4d5e6f708112", "assistant", "Hello!"),
          message("0196f6a4-7c1e-7d2a-9b3c-4d5e6f708113", "assistant", "", {
            status: "error",
            parts: [],
            error: "The provider ended the answer with an error.",
          }),
        ],
      ],
    ]);
    expect(html).toContain("Say hello");
    expect(html).toContain("Hello!");
    expect(html).toContain("The provider ended the answer with an error.");
  });
});
