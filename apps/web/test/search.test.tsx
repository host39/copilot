// Search: the query rules, the marked words of a snippet, links to a match, the request, and the
// screen.

import type { SearchHitResponse, UserResponse } from "@copilot/contracts";
import { afterEach, describe, expect, test } from "@rstest/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { resetTransport, setTransport } from "@web/api/client";
import { keys } from "@web/api/routes/keys";
import { search } from "@web/api/routes/search";
import { clearToken, setToken } from "@web/auth/token";
import {
  highlight,
  hitLink,
  messageAnchor,
  queryProblem,
  terms,
} from "@web/routes/search/highlight";
import SearchPage from "@web/routes/search/page";
import { renderToString } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";

const CHAT = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708192";
const MESSAGE = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708193";
const AT = "2026-11-26T10:00:00Z";

afterEach(() => {
  clearToken();
  resetTransport();
});

describe("search rules", () => {
  test("needs text, at most 200 characters", () => {
    expect(queryProblem("  ")).toBe("Enter text to search for.");
    expect(queryProblem("x".repeat(200))).toBeNull();
    expect(queryProblem("🦀".repeat(200))).toBeNull();
    expect(queryProblem("x".repeat(201))).toBe("Search text must be at most 200 characters.");
  });

  test("splits a query into distinct lowercase words, longest first", () => {
    expect(terms("Rust, rust book; C++ Übersicht")).toEqual(["übersicht", "rust", "book", "c"]);
    expect(terms("!!!")).toEqual([]);
  });

  test("marks the start of words, ignoring case, never inside a word", () => {
    expect(highlight("Rusty trust in RUST", ["rust"])).toEqual([
      { text: "Rust", match: true },
      { text: "y trust in ", match: false },
      { text: "RUST", match: true },
    ]);
    expect(highlight("a.b (c)", ["c"])).toEqual([
      { text: "a.b (", match: false },
      { text: "c", match: true },
      { text: ")", match: false },
    ]);
    expect(highlight("plain", [])).toEqual([{ text: "plain", match: false }]);
    expect(highlight("", ["x"])).toEqual([]);
  });

  test("a message match links to that message in its chat", () => {
    const hit: SearchHitResponse = {
      chatId: CHAT,
      chatTitle: "Plans",
      messageId: MESSAGE,
      snippet: "the plan",
    };
    expect(hitLink(hit)).toBe(`/chats/${CHAT}#${messageAnchor(MESSAGE)}`);
    expect(hitLink({ ...hit, messageId: null })).toBe(`/chats/${CHAT}`);
    expect(messageAnchor(MESSAGE)).toBe(`message-${MESSAGE}`);
  });

  test("sends the text and the limit", async () => {
    const urls: string[] = [];
    setTransport({
      server: () => "",
      token: () => "token",
      fetch: async (input) => {
        urls.push(input);
        return new Response(JSON.stringify({ items: [], total: 0 }), {
          headers: { "Content-Type": "application/json" },
        });
      },
    });
    expect(await search("rust book", 50)).toEqual([]);
    expect(urls).toEqual(["/api/search?q=rust+book&limit=50"]);
  });
});

describe("the search screen", () => {
  const user: UserResponse = {
    id: "0195d6a4-1b2c-7d3e-8f40-123456789abc",
    username: "ada",
    displayName: "Ada",
    role: "user",
    disabled: false,
    createdAt: AT,
    updatedAt: AT,
  };

  function render(url: string, data: [readonly unknown[], unknown][] = []) {
    const client = new QueryClient();
    setToken("token");
    client.setQueryData(["auth", "me", "token"], user);
    for (const [key, value] of data) client.setQueryData(key, value);
    return renderToString(
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={[url]}>
          <SearchPage />
        </MemoryRouter>
      </QueryClientProvider>
    );
  }

  test("lists title and message matches with the words marked", () => {
    const html = render("/search?q=rust", [
      [
        keys.search("rust"),
        [
          { chatId: CHAT, chatTitle: "Rust notes", messageId: null, snippet: "Rust notes" },
          {
            chatId: CHAT,
            chatTitle: "Rust notes",
            messageId: MESSAGE,
            snippet: "Ownership in Rust <b>is</b> strict",
          },
        ],
      ],
    ]);
    expect(html).toContain('value="rust"');
    expect(html).toContain("<mark");
    expect(html).toContain("Matched the title");
    expect(html).toContain(`href="/chats/${CHAT}#message-${MESSAGE}"`);
    expect(html).toContain("&lt;b&gt;is&lt;/b&gt;");
  });

  test("says when nothing matches, and what the screen is for", () => {
    expect(render("/search?q=zebra", [[keys.search("zebra"), []]])).toContain(
      "No chats match “zebra”."
    );
    expect(render("/search")).toContain("Find chats by their title or any message in them.");
  });

  test("refuses text that is too long before sending", () => {
    expect(render(`/search?q=${"x".repeat(201)}`)).toContain(
      "Search text must be at most 200 characters."
    );
  });
});
