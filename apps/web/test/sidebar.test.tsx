// The sidebar and chat lists: grouping by day, the Scheduled count, pages of chats, the rows'
// dots, the drawer shell, and the chat list helper.

import type { ChatSummaryResponse, ProjectResponse, UserResponse } from "@copilot/contracts";
import { afterEach, describe, expect, test } from "@rstest/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { resetTransport, setTransport } from "@web/api/client";
import { fetchChats } from "@web/api/routes/chats";
import { keys } from "@web/api/routes/keys";
import { clearToken, setToken } from "@web/auth/token";
import { ChatDots, chatTitle } from "@web/components/Chats/ChatRow";
import { flatten } from "@web/components/Chats/pages";
import { AppShell } from "@web/components/Layout/AppShell";
import { MenuButton } from "@web/components/Layout/Shell";
import { groupByDay, unreadBadge } from "@web/components/Sidebar/groups";
import { Sidebar } from "@web/components/Sidebar/Sidebar";
import { newChatLink, PROJECT_PARAM } from "@web/routes/home/draft";
import path from "@web/routes/path";
import { renderToString } from "react-dom/server";
import { MemoryRouter, Route, Routes } from "react-router-dom";

const AT = "2026-11-26T10:00:00Z";
const PROJECT = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708200";

let counter = 0;

function chat(title: string, updatedAt: string, extra: Partial<ChatSummaryResponse> = {}) {
  counter += 1;
  return {
    id: `0196f6a4-7c1e-7d2a-9b3c-${counter.toString().padStart(12, "0")}`,
    projectId: null,
    modelId: null,
    scheduleId: null,
    title,
    pinned: false,
    unread: false,
    awaitingApproval: false,
    createdAt: updatedAt,
    updatedAt,
    ...extra,
  } satisfies ChatSummaryResponse;
}

/** A local time on a day of November 2026. */
const local = (day: number, hour = 9) => new Date(2026, 10, day, hour).toISOString();

afterEach(() => {
  clearToken();
  resetTransport();
});

describe("recent chats by day", () => {
  const now = new Date(2026, 10, 27, 12);

  test("groups by local day, newest first, keeping the server's order within a day", () => {
    const morning = chat("Morning", local(27, 9));
    const late = chat("Late", local(27, 11));
    const groups = groupByDay(
      [
        late,
        morning,
        chat("Yesterday's", local(26, 22)),
        chat("Older", local(25)),
        chat("Last year", new Date(2025, 11, 31, 10).toISOString()),
      ],
      now,
      "en-US"
    );
    expect(groups.map((group) => [group.label, group.chats.map((item) => item.title)])).toEqual([
      ["Today", ["Late", "Morning"]],
      ["Yesterday", ["Yesterday's"]],
      ["Wednesday, Nov 25", ["Older"]],
      ["Wednesday, Dec 31, 2025", ["Last year"]],
    ]);
  });

  test("a chat dated after now counts as today", () => {
    const groups = groupByDay([chat("Ahead", local(28))], now, "en-US");
    expect(groups.map((group) => group.label)).toEqual(["Today"]);
  });

  test("the Scheduled count names unread runs, up to 99+", () => {
    expect(unreadBadge([])).toBeNull();
    expect(unreadBadge([{ unread: false }])).toBeNull();
    expect(unreadBadge([{ unread: true }, { unread: false }, { unread: true }])).toBe("2");
    expect(unreadBadge(Array.from({ length: 100 }, () => ({ unread: true })))).toBe("99+");
  });
});

describe("chat lists", () => {
  test("pages flatten without repeating a chat that moved up between them", () => {
    const first = chat("First", AT);
    const second = chat("Second", AT);
    expect(
      flatten([
        { items: [first, second], next: "c1" },
        { items: [second, chat("Third", AT)], next: null },
      ]).map((item) => item.title)
    ).toEqual(["First", "Second", "Third"]);
    expect(flatten(undefined)).toEqual([]);
  });

  test("an untitled chat reads New chat", () => {
    expect(chatTitle({ title: "  " })).toBe("New chat");
    expect(chatTitle({ title: "Plans" })).toBe("Plans");
  });

  test("approval outranks unread, and each dot has a name", () => {
    const approval = renderToString(<ChatDots chat={{ unread: true, awaitingApproval: true }} />);
    expect(approval).toContain("Needs approval");
    expect(approval).not.toContain("Unread");
    expect(renderToString(<ChatDots chat={{ unread: true, awaitingApproval: false }} />)).toContain(
      "Unread"
    );
    expect(renderToString(<ChatDots chat={{ unread: false, awaitingApproval: false }} />)).toBe("");
  });

  test("fetches a page with its filters and cursor", async () => {
    const urls: string[] = [];
    setTransport({
      server: () => "",
      token: () => "token",
      fetch: async (input) => {
        urls.push(input);
        return new Response(JSON.stringify({ items: [], next: null }), {
          headers: { "Content-Type": "application/json" },
        });
      },
    });
    expect(await fetchChats({ scheduled: false, pinned: false, cursor: "abc", limit: 25 })).toEqual(
      { items: [], next: null }
    );
    expect(urls).toEqual(["/api/chats?scheduled=false&pinned=false&cursor=abc&limit=25"]);
  });

  test("a project's new chat link carries the project", () => {
    expect(newChatLink(PROJECT)).toBe(`/?${PROJECT_PARAM}=${PROJECT}`);
  });
});

const user: UserResponse = {
  id: "0195d6a4-1b2c-7d3e-8f40-123456789abc",
  username: "ada",
  displayName: "Ada",
  role: "user",
  disabled: false,
  createdAt: AT,
  updatedAt: AT,
};

const project = (name: string, extra: Partial<ProjectResponse> = {}): ProjectResponse => ({
  id: PROJECT,
  name,
  icon: "",
  instructions: "",
  defaultModelId: null,
  reasoning: null,
  builtinTools: [],
  toolServerIds: [],
  pinned: false,
  createdAt: AT,
  updatedAt: AT,
  ...extra,
});

describe("the sidebar", () => {
  function render(data: [readonly unknown[], unknown][], element = <Sidebar />) {
    const client = new QueryClient();
    setToken("token");
    client.setQueryData(["auth", "me", "token"], user);
    for (const [key, value] of data) client.setQueryData(key, value);
    return renderToString(
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={["/"]}>{element}</MemoryRouter>
      </QueryClientProvider>
    );
  }

  const pages = (items: ChatSummaryResponse[], next: string | null = null) => ({
    pages: [{ items, next }],
    pageParams: [null],
  });

  test("shows projects, pinned chats, recent chats by day, dots, and the unread runs", () => {
    const today = new Date().toISOString();
    const html = render([
      [keys.projectList({ pinned: true }), [project("Pinned work", { pinned: true, icon: "📌" })]],
      [
        keys.projectList({ pinned: false, limit: 5 }),
        [project("Research", { id: "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708201" })],
      ],
      [keys.chatList({ pinned: true }), pages([chat("Favorite", today, { pinned: true })])],
      [
        keys.chatList({ scheduled: false, pinned: false }),
        pages(
          [
            chat("Fresh", today, { unread: true }),
            chat("Waiting", today, { awaitingApproval: true }),
            chat("", "2025-01-02T10:00:00Z"),
          ],
          "next-page"
        ),
      ],
      [
        keys.runs,
        { items: [chat("Run", today, { unread: true }), chat("Seen", today)], next: null },
      ],
    ]);
    expect(html).toContain("📌");
    expect(html).toContain("Pinned work");
    expect(html).toContain("Research");
    expect(html).toContain(`aria-label="New chat in Research"`);
    expect(html).toContain(`href="/?project=${PROJECT}"`);
    expect(html).toContain("All projects");
    expect(html).toContain("Favorite");
    expect(html).toContain(">Today</h3>");
    expect(html).toContain("Fresh");
    expect(html).toContain("Needs approval");
    expect(html).toContain("Unread");
    expect(html).toContain("New chat</span>");
    expect(html).toContain("2025");
    expect(html).toContain("Show more");
    expect(html).toMatch(/>1<span class="sr-only"> unread<\/span>/);
    expect(html).toContain("Sign out");
  });

  test("without chats it says so and hides the pinned section", () => {
    const html = render([
      [keys.chatList({ pinned: true }), pages([])],
      [keys.chatList({ scheduled: false, pinned: false }), pages([])],
    ]);
    expect(html).toContain("No chats yet.");
    expect(html).not.toContain(">Pinned</h2>");
  });

  test("the shell has a drawer and a menu button on phones", () => {
    const client = new QueryClient();
    setToken("token");
    client.setQueryData(["auth", "me", "token"], user);
    const html = renderToString(
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={["/"]}>
          <Routes>
            <Route element={<AppShell />}>
              <Route path={path.home} element={<p>Home</p>} />
            </Route>
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );
    expect(html).toContain('<dialog id="drawer"');
    expect(html).toContain('aria-label="Open menu"');
    expect(html).toContain("Home");
  });

  test("a page's own menu button needs the shell", () => {
    expect(renderToString(<MenuButton />)).toBe("");
  });
});
