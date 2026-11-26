// Routing, guards, themes, and the helpers screens share.

import { readFileSync } from "node:fs";
import type { UserResponse } from "@copilot/contracts";
import { afterEach, beforeEach, describe, expect, rs, test } from "@rstest/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ApiError, apiRequest, resetTransport, setTransport } from "@web/api/client";
import { describeError } from "@web/api/errors";
import { shouldRetry } from "@web/api/query";
import { GuestOnly, RequireAdmin, RequireSession } from "@web/auth/guards";
import { clearToken, setToken } from "@web/auth/token";
import { fieldErrors } from "@web/forms";
import { installNativeTransport } from "@web/native";
import path, { link } from "@web/routes/path";
import { colorScheme, readTheme, saveTheme } from "@web/theme";
import type { ReactNode } from "react";
import { renderToString } from "react-dom/server";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { z } from "zod";

const nativeFetch = rs.fn(
  async (_input: string, _init?: RequestInit) => new Response(null, { status: 204 })
);
rs.mock("@tauri-apps/plugin-http", () => ({ fetch: nativeFetch }));

const user = (role: UserResponse["role"]): UserResponse => ({
  id: "0195d6a4-1b2c-7d3e-8f40-123456789abc",
  username: "ada",
  displayName: "Ada",
  role,
  disabled: false,
  createdAt: "2026-11-26T10:00:00Z",
  updatedAt: "2026-11-26T10:00:00Z",
});

/** Renders `routes` at `url` with the session of `signedIn` (or none). */
function render(url: string, routes: ReactNode, signedIn?: UserResponse): string {
  const client = new QueryClient();
  if (signedIn) {
    setToken("token");
    client.setQueryData(["auth", "me", "token"], signedIn);
  } else {
    clearToken();
  }
  return renderToString(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[url]}>
        <Routes>{routes}</Routes>
      </MemoryRouter>
    </QueryClientProvider>
  );
}

afterEach(() => {
  clearToken();
  resetTransport();
  Reflect.deleteProperty(globalThis, "isTauri");
});

describe("paths", () => {
  test("the route table maps every path constant", () => {
    const table = readFileSync("src/routes/index.tsx", "utf8");
    const missing = Object.keys(path).filter((name) => !table.includes(`path.${name}}`));
    expect(missing).toEqual([]);
  });

  test("every path is distinct", () => {
    const values = Object.values(path);
    expect(new Set(values).size).toBe(values.length);
  });

  test("links encode ids", () => {
    expect(link.chat("a/b?c")).toBe("/chats/a%2Fb%3Fc");
    expect(link.provider("p1")).toBe("/settings/providers/p1");
  });
});

describe("guards", () => {
  const protectedRoutes = (
    <>
      <Route element={<RequireSession />}>
        <Route path="/chats/:chatId" element={<p>secret chat</p>} />
      </Route>
      <Route path="/login" element={<p>sign-in form</p>} />
    </>
  );

  test("signed-out users never see signed-in screens", () => {
    expect(render("/chats/1", protectedRoutes)).not.toContain("secret chat");
  });

  test("signed-in users see them", () => {
    expect(render("/chats/1", protectedRoutes, user("user"))).toContain("secret chat");
  });

  test("the sign-in screens are for signed-out users only", () => {
    const routes = (
      <Route element={<GuestOnly />}>
        <Route path="/login" element={<p>sign-in form</p>} />
      </Route>
    );
    expect(render("/login", routes)).toContain("sign-in form");
    expect(render("/login", routes, user("user"))).not.toContain("sign-in form");
  });

  test("only administrators see the Users screens", () => {
    const routes = (
      <Route element={<RequireAdmin />}>
        <Route path="/settings/users" element={<p>user list</p>} />
      </Route>
    );
    expect(render("/settings/users", routes, user("administrator"))).toContain("user list");
    const denied = render("/settings/users", routes, user("manager"));
    expect(denied).not.toContain("user list");
    expect(denied).toContain("Page not found");
  });
});

describe("themes", () => {
  const items = new Map<string, string>();

  beforeEach(() => {
    items.clear();
    Object.defineProperty(globalThis, "localStorage", {
      configurable: true,
      value: {
        getItem: (key: string) => items.get(key) ?? null,
        setItem: (key: string, value: string) => items.set(key, value),
        removeItem: (key: string) => items.delete(key),
      },
    });
  });

  test("system follows the device; light and dark pin a scheme", () => {
    expect(colorScheme("system")).toBe("light dark");
    expect(colorScheme("light")).toBe("light");
    expect(colorScheme("dark")).toBe("dark");
  });

  test("stores a pinned scheme and forgets it for system", () => {
    saveTheme("dark");
    expect(items.get("copilot.color-scheme")).toBe("dark");
    expect(readTheme()).toBe("dark");
    saveTheme("system");
    expect(items.has("copilot.color-scheme")).toBe(false);
    expect(readTheme()).toBe("system");
  });

  test("reads an unknown stored value as system", () => {
    items.set("copilot.color-scheme", "sepia");
    expect(readTheme()).toBe("system");
  });

  test("the first-paint script reads the same key", () => {
    expect(readFileSync("public/theme.js", "utf8")).toContain('"copilot.color-scheme"');
  });
});

describe("native transport", () => {
  test("web builds keep the browser's fetch", async () => {
    await installNativeTransport();
    setTransport({ server: () => "" });
    const browserFetch = rs
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(new Response(null, { status: 204 }));
    await apiRequest("/settings");
    expect(browserFetch).toHaveBeenCalledTimes(1);
    expect(nativeFetch).not.toHaveBeenCalled();
    browserFetch.mockRestore();
  });

  test("native builds send requests through plugin-http", async () => {
    Object.defineProperty(globalThis, "isTauri", { value: true, configurable: true });
    await installNativeTransport();
    setTransport({ server: () => "https://copilot.example" });
    await apiRequest("/settings");
    expect(nativeFetch).toHaveBeenCalledTimes(1);
    expect(nativeFetch.mock.calls[0]?.[0]).toBe("https://copilot.example/api/settings");
  });
});

describe("helpers", () => {
  test("queries retry server failures but not answers the server meant", () => {
    const answer = (status: number) => new ApiError(status, { error: "x" });
    expect(shouldRetry(0, answer(401))).toBe(false);
    expect(shouldRetry(0, answer(404))).toBe(false);
    expect(shouldRetry(0, answer(429))).toBe(false);
    expect(shouldRetry(0, answer(503))).toBe(true);
    expect(shouldRetry(1, answer(0))).toBe(true);
    expect(shouldRetry(2, answer(0))).toBe(false);
  });

  test("describes API errors with their message, field, and wait", () => {
    const error = new ApiError(
      429,
      {
        error: "Try again in 15 minutes.",
        code: "ACCOUNT_LOCKED",
        details: { retryAfterSecs: 900 },
      },
      900
    );
    expect(describeError(error)).toEqual({
      message: "Try again in 15 minutes.",
      field: undefined,
      retryAfterSecs: 900,
    });
    expect(describeError(new Error("boom")).message).toBe("Something went wrong. Try again.");
  });

  test("keeps the first message per field", () => {
    const schema = z.object({
      name: z.string().min(3, "Too short.").regex(/^a/, "Starts with a."),
      age: z.number(),
    });
    const result = schema.safeParse({ name: "b", age: "x" });
    expect(result.success).toBe(false);
    if (!result.success) {
      expect(Object.keys(fieldErrors(result.error)).sort()).toEqual(["age", "name"]);
      expect(fieldErrors(result.error).name).toBe("Too short.");
    }
  });
});
