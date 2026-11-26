// The sign-in, setup, and register screens follow rustauth's setup status, and the Connect
// screen's warning for unencrypted servers.

import type { SetupStatusResponse } from "@copilot/contracts";
import { afterEach, describe, expect, test } from "@rstest/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { clearToken } from "@web/auth/token";
import { isPrivateHost, needsHttpWarning } from "@web/config";
import LoginPage from "@web/routes/auth/login/page";
import RegisterPage from "@web/routes/auth/register/page";
import SetupPage from "@web/routes/auth/setup/page";
import type { ReactNode } from "react";
import { renderToString } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";

/** Renders `page` signed out, with `GET /auth/setup` answering `status` (lowercased: HTML attribute names ignore case). */
function render(page: ReactNode, status: SetupStatusResponse): string {
  clearToken();
  const client = new QueryClient();
  client.setQueryData(["auth", "setup"], status);
  return renderToString(
    <QueryClientProvider client={client}>
      <MemoryRouter>{page}</MemoryRouter>
    </QueryClientProvider>
  ).toLowerCase();
}

const fresh = { needsSetup: true, registrationOpen: false };
const open = { needsSetup: false, registrationOpen: true };
const closed = { needsSetup: false, registrationOpen: false };

afterEach(() => {
  clearToken();
});

describe("sign-in", () => {
  test("offers sign-up only while registration is open", () => {
    expect(render(<LoginPage />, open)).toContain("create an account");
    expect(render(<LoginPage />, closed)).not.toContain("create an account");
  });

  test("uses the password manager's saved password", () => {
    const html = render(<LoginPage />, closed);
    expect(html).toContain('autocomplete="username"');
    expect(html).toContain('autocomplete="current-password"');
  });

  test("sends a server without accounts to setup", () => {
    expect(render(<LoginPage />, fresh)).not.toContain("current-password");
  });
});

describe("first-run setup", () => {
  test("asks for the setup token while no account exists", () => {
    const html = render(<SetupPage />, fresh);
    expect(html).toContain("setup token");
    expect(html).toContain('autocomplete="new-password"');
  });

  test("is closed once the administrator exists", () => {
    expect(render(<SetupPage />, closed)).not.toContain("setup token");
  });
});

describe("register", () => {
  test("shows the form while registration is open, without a setup token", () => {
    const html = render(<RegisterPage />, open);
    expect(html).toContain("create account");
    expect(html).toContain('autocomplete="new-password"');
    expect(html).not.toContain("setup token");
  });

  test("explains that registration is closed", () => {
    const html = render(<RegisterPage />, closed);
    expect(html).toContain("does not accept new accounts");
    expect(html).not.toContain("new-password");
  });

  test("sends a server without accounts to setup", () => {
    expect(render(<RegisterPage />, fresh)).not.toContain("new-password");
  });
});

describe("unencrypted servers", () => {
  test("loopback and private addresses count as private", () => {
    for (const host of [
      "localhost",
      "copilot.localhost",
      "127.0.0.1",
      "10.1.2.3",
      "172.16.0.1",
      "172.31.255.255",
      "192.168.1.20",
      "100.64.0.1",
      "[::1]",
      "[fd12:3456::1]",
    ]) {
      expect([host, isPrivateHost(host)]).toEqual([host, true]);
    }
  });

  test("public addresses and names do not", () => {
    for (const host of [
      "copilot.example.com",
      "8.8.8.8",
      "172.32.0.1",
      "192.169.0.1",
      "100.128.0.1",
      "[2001:db8::1]",
      "fc.example.com",
    ]) {
      expect([host, isPrivateHost(host)]).toEqual([host, false]);
    }
  });

  test("plain http to a public host needs consent; https and private hosts do not", () => {
    expect(needsHttpWarning("http://copilot.example.com")).toBe(true);
    expect(needsHttpWarning("https://copilot.example.com")).toBe(false);
    expect(needsHttpWarning("http://192.168.1.20:3000")).toBe(false);
    expect(needsHttpWarning("http://localhost:3000")).toBe(false);
  });
});
