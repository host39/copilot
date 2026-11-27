// Settings, Users (administrators only): account badges and lockouts, temporary passwords, the
// profile and reset forms, and the screens.

import { createUserRequestSchema, type UserResponse } from "@copilot/contracts";
import { afterEach, describe, expect, test } from "@rstest/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { keys } from "@web/api/routes/keys";
import { clearToken, setToken } from "@web/auth/token";
import { fieldErrors } from "@web/forms";
import path from "@web/routes/path";
import {
  accountBadges,
  isLocked,
  profileSchema,
  resetSchema,
  temporaryPassword,
} from "@web/routes/settings/user/accounts";
import UserPage from "@web/routes/settings/user/page";
import UsersPage from "@web/routes/settings/users/page";
import { renderToString } from "react-dom/server";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import type { z } from "zod";

const AT = "2026-11-26T10:00:00Z";
const NOW = Date.parse(AT);
const ADMIN = "0195d6a4-1b2c-7d3e-8f40-123456789abc";
const GRACE = "0195d6a4-1b2c-7d3e-8f40-123456789abd";

function account(id: string, extra: Partial<UserResponse> = {}): UserResponse {
  return {
    id,
    username: id === ADMIN ? "ada" : "grace",
    displayName: id === ADMIN ? "Ada" : "Grace",
    role: id === ADMIN ? "administrator" : "user",
    disabled: false,
    createdAt: AT,
    updatedAt: AT,
    ...extra,
  };
}

function parse<S extends z.ZodType>(schema: S, input: unknown) {
  const parsed = schema.safeParse(input);
  return parsed.success ? { value: parsed.data } : { errors: fieldErrors(parsed.error) };
}

afterEach(() => {
  clearToken();
});

describe("account states", () => {
  test("an account is locked only until its lockout ends", () => {
    expect(isLocked(account(GRACE), NOW)).toBe(false);
    expect(isLocked(account(GRACE, { lockedUntil: "2026-11-26T10:15:00Z" }), NOW)).toBe(true);
    expect(isLocked(account(GRACE, { lockedUntil: "2026-11-26T09:45:00Z" }), NOW)).toBe(false);
  });

  test("badges name the role above user, and disabled and locked accounts", () => {
    const labels = (user: UserResponse) => accountBadges(user, NOW).map((badge) => badge.label);
    expect(labels(account(ADMIN))).toEqual(["Administrator"]);
    expect(labels(account(GRACE))).toEqual([]);
    expect(
      labels(
        account(GRACE, { role: "manager", disabled: true, lockedUntil: "2026-11-26T11:00:00Z" })
      )
    ).toEqual(["Manager", "Disabled", "Locked"]);
  });
});

describe("temporary passwords", () => {
  test("are four groups of four unambiguous letters and digits that rustauth accepts", () => {
    for (let round = 0; round < 50; round++) {
      const password = temporaryPassword();
      expect(password).toMatch(/^[a-hjkmnp-zA-HJKMNP-Z2-9]{4}(-[a-hjkmnp-zA-HJKMNP-Z2-9]{4}){3}$/);
      expect(
        createUserRequestSchema.safeParse({
          username: "grace",
          displayName: "Grace",
          temporaryPassword: password,
          role: "user",
        }).success
      ).toBe(true);
    }
  });

  test("skip random bytes that would make some characters likelier", () => {
    // 54 characters: bytes from 216 up are dropped, so 255 never counts and 0 maps to "a".
    let call = 0;
    const random = (count: number) => {
      call += 1;
      return new Uint8Array(count).fill(call === 1 ? 255 : 0);
    };
    expect(temporaryPassword(random)).toBe("aaaa-aaaa-aaaa-aaaa");
    expect(call).toBe(2);
  });
});

describe("the account forms", () => {
  const grace = account(GRACE);

  test("the profile form sends only what changed", () => {
    expect(parse(profileSchema(grace), { displayName: "Grace", role: "user" })).toEqual({
      value: {},
    });
    expect(parse(profileSchema(grace), { displayName: " Grace H. ", role: "manager" })).toEqual({
      value: { displayName: "Grace H.", role: "manager" },
    });
  });

  test("the profile form keeps rustauth's rules", () => {
    expect(parse(profileSchema(grace), { displayName: " ", role: "user" })).toEqual({
      errors: { displayName: "Display name is required." },
    });
  });

  test("a reset password has at least 12 characters", () => {
    expect(parse(resetSchema, { password: "short" })).toEqual({
      errors: { password: "Password must be at least 12 characters long." },
    });
    expect(parse(resetSchema, { password: "abcd-efgh-jkmn-pqrs" })).toEqual({
      value: { password: "abcd-efgh-jkmn-pqrs" },
    });
  });
});

describe("the user screens", () => {
  function render(url: string, data: [readonly unknown[], unknown][]) {
    const client = new QueryClient();
    setToken("token");
    client.setQueryData(["auth", "me", "token"], account(ADMIN));
    for (const [key, value] of data) client.setQueryData(key, value);
    return renderToString(
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={[url]}>
          <Routes>
            <Route path={path.users} element={<UsersPage />} />
            <Route path={path.userNew} element={<UserPage />} />
            <Route path={path.user} element={<UserPage />} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );
  }

  test("lists accounts with the caller marked and their states", () => {
    const html = render(path.users, [
      [
        keys.usersPage(0),
        { items: [account(ADMIN), account(GRACE, { disabled: true })], total: 2 },
      ],
    ]);
    expect(html).toContain(">You<");
    expect(html).toContain("@grace");
    expect(html).toContain(">Disabled<");
    expect(html).toContain(`href="/settings/users/${GRACE}"`);
    expect(html).toContain(`href="${path.userNew}"`);
    expect(html).not.toContain("Next");
  });

  test("pages through more than 100 accounts", () => {
    const html = render(path.users, [[keys.usersPage(0), { items: [account(ADMIN)], total: 250 }]]);
    expect(html).toContain("1 to 100 of 250");
    expect(html).toContain("Next");
  });

  test("the add screen asks for a name, username, temporary password, and role", () => {
    const html = render(path.userNew, []);
    expect(html).toContain('name="displayName"');
    expect(html).toContain('name="username"');
    expect(html).toContain('name="password"');
    expect(html).toContain("Generate");
    expect(html).toContain("Administrator");
  });

  test("an account offers profile, access, reset, and delete", () => {
    const html = render(`/settings/users/${GRACE}`, [
      [
        keys.user(GRACE),
        account(GRACE, {
          disabled: true,
          lockedUntil: new Date(Date.now() + 600_000).toISOString(),
        }),
      ],
    ]);
    expect(html).toContain('value="Grace"');
    expect(html).toContain("Enable account");
    expect(html).toContain("Unlock");
    expect(html).toContain("Reset Password");
    expect(html).toContain("Delete User");
    expect(html).not.toContain("This is your account");
  });

  test("warns administrators that changing their own account signs them out", () => {
    const html = render(`/settings/users/${ADMIN}`, [[keys.user(ADMIN), account(ADMIN)]]);
    expect(html).toContain("This is your account");
    expect(html).toContain("Disable account");
    expect(html).not.toContain(">Unlock<");
  });
});
