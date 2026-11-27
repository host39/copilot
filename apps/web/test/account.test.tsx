// Settings, Account: the password form's rules, device names and order, the shared formatting, and
// the screen itself.

import type { SessionResponse, UserResponse } from "@copilot/contracts";
import { afterEach, describe, expect, test } from "@rstest/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { keys } from "@web/api/routes/keys";
import { clearToken, setToken } from "@web/auth/token";
import { fieldErrors } from "@web/forms";
import { deviceName, sortSessions } from "@web/routes/settings/account/devices";
import AccountPage from "@web/routes/settings/account/page";
import { passwordSchema } from "@web/routes/settings/account/password";
import { ago, plural } from "@web/routes/settings/format";
import { renderToString } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";

const AT = "2026-11-26T10:00:00Z";
const NOW = Date.parse(AT);

const user: UserResponse = {
  id: "0195d6a4-1b2c-7d3e-8f40-123456789abc",
  username: "ada",
  displayName: "Ada",
  role: "user",
  disabled: false,
  createdAt: AT,
  updatedAt: AT,
};

function session(id: string, extra: Partial<SessionResponse> = {}): SessionResponse {
  return {
    id,
    current: false,
    methods: ["password"],
    assurance: "single",
    createdAt: AT,
    lastUsedAt: AT,
    expiresAt: AT,
    userAgent: "",
    ...extra,
  };
}

afterEach(() => {
  clearToken();
});

describe("the password form", () => {
  test("needs the current password, rustauth's rules, and a matching repeat", () => {
    const parsed = passwordSchema.safeParse({ current: "", new: "short", confirm: "other" });
    expect(parsed.success).toBe(false);
    if (parsed.success) return;
    const errors = fieldErrors(parsed.error);
    expect(errors.current).toBe("Enter your current password.");
    expect(errors.new).toBe("Password must be at least 12 characters long.");
  });

  test("a repeat that differs is an error on the repeat", () => {
    const parsed = passwordSchema.safeParse({
      current: "old password",
      new: "a new long password",
      confirm: "a new long passwort",
    });
    expect(parsed.success).toBe(false);
    if (parsed.success) return;
    expect(fieldErrors(parsed.error)).toEqual({ confirm: "The passwords do not match." });
  });

  test("sends only the current and the new password", () => {
    expect(
      passwordSchema.parse({
        current: "old password",
        new: "a new long password",
        confirm: "a new long password",
      })
    ).toEqual({ current: "old password", new: "a new long password" });
  });
});

describe("devices", () => {
  test("names browsers and systems", () => {
    const agents: [string, string][] = [
      [
        "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0",
        "Firefox on Linux",
      ],
      [
        "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Mobile Safari/537.36",
        "Chrome on Android",
      ],
      [
        "Mozilla/5.0 (iPhone; CPU iPhone OS 19_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/19.0 Mobile/15E148 Safari/604.1",
        "Safari on iOS",
      ],
      [
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Safari/537.36 Edg/141.0",
        "Edge on Windows",
      ],
      [
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/19.0 Safari/605.1.15",
        "Safari on macOS",
      ],
    ];
    for (const [agent, name] of agents) expect(deviceName(agent)).toBe(name);
  });

  test("shows unknown agents as themselves, shortened, and empty ones as unknown", () => {
    expect(deviceName("curl/8.9.1")).toBe("curl/8.9.1");
    expect(deviceName("x".repeat(80))).toHaveLength(60);
    expect(deviceName("  ")).toBe("Unknown device");
  });

  test("lists this device first, then the most recently used", () => {
    const sorted = sortSessions([
      session("old", { lastUsedAt: "2026-11-01T10:00:00Z" }),
      session("new", { lastUsedAt: "2026-11-25T10:00:00Z" }),
      session("this", { current: true, lastUsedAt: "2026-10-01T10:00:00Z" }),
    ]);
    expect(sorted.map((item) => item.id)).toEqual(["this", "new", "old"]);
  });
});

describe("formatting", () => {
  test("says how long ago something happened", () => {
    expect(ago(AT, NOW + 20_000)).toBe("just now");
    expect(ago(AT, NOW - 60_000)).toBe("just now");
    expect(ago(AT, NOW + 5 * 60_000)).toBe("5 minutes ago");
    expect(ago(AT, NOW + 3 * 3_600_000)).toBe("3 hours ago");
    expect(ago(AT, NOW + 24 * 3_600_000)).toBe("yesterday");
    expect(ago(AT, NOW + 4 * 24 * 3_600_000)).toBe("4 days ago");
    expect(ago(AT, NOW + 45 * 24 * 3_600_000)).not.toContain("ago");
    expect(ago("not a date", NOW)).toBe("");
  });

  test("counts with the right noun", () => {
    expect(plural(1, "chat")).toBe("1 chat");
    expect(plural(3, "chat")).toBe("3 chats");
    expect(plural(0, "match", "matches")).toBe("0 matches");
  });
});

describe("the account screen", () => {
  function render(): string {
    const client = new QueryClient();
    setToken("token");
    client.setQueryData(["auth", "me", "token"], user);
    client.setQueryData(keys.settings, { instructions: "Answer in English." });
    client.setQueryData(keys.sessions, [
      session("a", {
        current: true,
        userAgent: "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0",
      }),
      session("b", {
        userAgent:
          "Mozilla/5.0 (Linux; Android 15) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Mobile Safari/537.36",
        ip: "192.0.2.10",
      }),
    ]);
    return renderToString(
      <QueryClientProvider client={client}>
        <MemoryRouter>
          <AccountPage />
        </MemoryRouter>
      </QueryClientProvider>
    );
  }

  test("shows the profile, password, instructions, appearance, and devices", () => {
    const html = render();
    for (const heading of [
      "Profile",
      "Password",
      "Personal Instructions",
      "Appearance",
      "Devices",
    ]) {
      expect(html).toContain(`>${heading}</h2>`);
    }
    expect(html).toContain('value="Ada"');
    expect(html).toContain("@ada");
    expect(html).toContain("Answer in English.");
  });

  test("lists the devices with this one marked and offers to sign out the others", () => {
    const html = render();
    expect(html).toContain("Firefox on Linux");
    expect(html).toContain("This device");
    expect(html).toContain("Chrome on Android");
    expect(html).toContain("192.0.2.10");
    expect(html).toContain("Sign out other devices");
  });

  test("password fields use the password manager's autocomplete", () => {
    // HTML attribute names ignore case; React writes `autoComplete`.
    const html = render().toLowerCase();
    expect(html).toContain('autocomplete="current-password"');
    expect(html.match(/autocomplete="new-password"/g)).toHaveLength(2);
  });
});
