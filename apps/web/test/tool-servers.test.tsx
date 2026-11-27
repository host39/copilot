// Settings, Tool servers: the credential rules (mirroring the server's), the add and settings
// forms, tool modes and what a test changed, adding with a test, and the screens.

import type { ServerTool, ToolServerResponse, UserResponse } from "@copilot/contracts";
import { afterEach, describe, expect, test } from "@rstest/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { resetTransport, setTransport } from "@web/api/client";
import { keys } from "@web/api/routes/keys";
import { clearToken, setToken } from "@web/auth/token";
import { fieldErrors } from "@web/forms";
import SignInDonePage from "@web/routes/oauth-done/page";
import path from "@web/routes/path";
import {
  addServer,
  clientProblem,
  clientSchema,
  credentials,
  credentialsProblem,
  credentialsSchema,
  type HeaderRow,
  NO_CREDENTIALS,
  serverSchema,
  settingsSchema,
} from "@web/routes/settings/tool-server/forms";
import {
  modeFor,
  modesBody,
  modesChanged,
  sortTools,
  testSummary,
} from "@web/routes/settings/tool-server/modes";
import ToolServerPage from "@web/routes/settings/tool-server/page";
import {
  doneMessage,
  needsSignIn,
  openSignIn,
  signInBadge,
} from "@web/routes/settings/tool-server/sign-in";
import {
  commandLine,
  location,
  type ProgramInput,
  parseArgs,
  programPatch,
  programProblem,
  programSchema,
  quote,
} from "@web/routes/settings/tool-server/stdio";
import ToolServersPage from "@web/routes/settings/tool-servers/page";
import { renderToString } from "react-dom/server";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import type { z } from "zod";

const AT = "2026-11-26T10:00:00Z";
const ID = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708100";

function tool(name: string, extra: Partial<ServerTool> = {}): ServerTool {
  return { name, title: null, description: "", inputSchema: {}, readOnly: false, ...extra };
}

const server: ToolServerResponse = {
  id: ID,
  name: "Docs",
  slug: "docs",
  transport: "http",
  url: "https://mcp.example.com/mcp",
  stdio: null,
  approval: "ask",
  toolModes: { search: "auto" },
  enabled: true,
  credentials: "set",
  authMode: "headers",
  oauth: null,
  protocolVersion: "2026-07-28",
  tools: [tool("search", { title: "Search Docs", readOnly: true }), tool("delete_page")],
  toolsFetchedAt: AT,
  reviewRequired: false,
  createdAt: AT,
  updatedAt: AT,
};

let next = 0;
function row(name: string, value: string): HeaderRow {
  next += 1;
  return { id: next, name, value };
}

function parse<S extends z.ZodType>(schema: S, input: unknown) {
  const parsed = schema.safeParse(input);
  return parsed.success ? { value: parsed.data } : { errors: fieldErrors(parsed.error) };
}

/** Whether the radio input named `name` with `value` renders checked. */
function checked(html: string, name: string, value: string): boolean {
  const inputs = html.match(/<input[^>]*>/g) ?? [];
  return inputs.some(
    (input) =>
      input.includes(`name="${name}"`) &&
      input.includes(`value="${value}"`) &&
      input.includes('checked=""')
  );
}

afterEach(() => {
  clearToken();
  resetTransport();
});

describe("credentials", () => {
  const problem = (bearer: string, ...rows: HeaderRow[]) =>
    credentialsProblem(bearer, rows)?.message;

  test("header names are HTTP tokens the MCP client does not set itself", () => {
    expect(problem("", row("X Team", "a"))).toBe(
      "Header names may contain only letters, digits, and !#$%&'*+-.^_`|~."
    );
    expect(problem("", row("Content-Type", "a"))).toBe(
      "The Content-Type header is set by Copilot."
    );
    expect(problem("", row("Mcp-Session-Id", "a"))).toBe(
      "The Mcp-Session-Id header is set by Copilot."
    );
    expect(problem("", row("X-Team", "a"))).toBeUndefined();
  });

  test("a header is listed once, ignoring case, with a one-line value", () => {
    expect(problem("", row("X-Key", "a"), row("x-key", "b"))).toBe(
      "The x-key header is listed twice."
    );
    expect(problem("", row("X-Key", " "))).toBe("Enter a value for the X-Key header.");
    expect(problem("", row("X-Key", "a\nb"))).toBe("The X-Key header must be a single line.");
    expect(problem("", row("X-Key", "a\tb"))).toBeUndefined();
  });

  test("a bearer token excludes a custom Authorization header and has no spaces", () => {
    expect(problem("token", row("authorization", "Basic abc"))).toBe(
      "Set either a bearer token or a custom Authorization header, not both."
    );
    expect(problem("", row("Authorization", "Basic abc"))).toBeUndefined();
    expect(problem("two words")).toBe("The bearer token must not contain spaces or line breaks.");
    expect(credentialsProblem("two words", [])?.field).toBe("bearer");
  });

  test("blank rows and a blank token are left out", () => {
    expect(credentials(" ", [row("", ""), row(" X-Team ", " a ")])).toEqual({
      headers: { "X-Team": "a" },
    });
    expect(credentials(" tok ", [])).toEqual({ bearer: "tok", headers: {} });
    expect(parse(credentialsSchema, { bearer: "", rows: [row("Host", "x")] })).toEqual({
      errors: { headers: "The Host header is set by Copilot." },
    });
  });
});

const oauthServer: ToolServerResponse = {
  ...server,
  credentials: "missing",
  authMode: "oauth",
  oauth: { state: "needs_sign_in", clientId: null, clientSecret: "missing", expiresAt: null },
};

describe("the server forms", () => {
  const input = {
    name: "Docs",
    slug: "",
    url: "https://mcp.example.com/mcp",
    approval: "ask",
    auth: "none",
    bearer: "",
    rows: [],
    clientId: "",
    clientSecret: "",
  };

  test("adding needs a name and an http address; the slug is short", () => {
    expect(parse(serverSchema, { ...input, name: "", url: "mcp.example.com" })).toEqual({
      errors: { name: "Enter a name.", url: "Enter a full http:// or https:// address." },
    });
    expect(parse(serverSchema, { ...input, slug: "Docs-1" })).toEqual({
      errors: { slug: "The slug must be 1 to 12 lowercase letters or digits." },
    });
  });

  test("adding sends the credentials, and a slug or token only when given", () => {
    expect(parse(serverSchema, input)).toEqual({
      value: {
        name: "Docs",
        url: "https://mcp.example.com/mcp",
        approval: "ask",
        authMode: "none",
        headers: {},
      },
    });
    expect(
      parse(serverSchema, {
        ...input,
        slug: "docs2",
        approval: "auto",
        auth: "headers",
        bearer: "tok",
        rows: [row("X-Team", "a")],
      })
    ).toEqual({
      value: {
        name: "Docs",
        url: "https://mcp.example.com/mcp",
        approval: "auto",
        authMode: "headers",
        headers: { "X-Team": "a" },
        slug: "docs2",
        bearer: "tok",
      },
    });
  });

  test("only the chosen authentication's fields are sent, and headers mode needs some", () => {
    expect(parse(serverSchema, { ...input, auth: "headers" })).toEqual({
      errors: { bearer: NO_CREDENTIALS },
    });
    expect(parse(serverSchema, { ...input, bearer: "tok", clientId: "client" })).toEqual({
      value: {
        name: "Docs",
        url: "https://mcp.example.com/mcp",
        approval: "ask",
        authMode: "none",
        headers: {},
      },
    });
    expect(
      parse(serverSchema, {
        ...input,
        auth: "oauth",
        bearer: "tok",
        clientId: " client-1 ",
        clientSecret: " s3cret ",
      })
    ).toEqual({
      value: {
        name: "Docs",
        url: "https://mcp.example.com/mcp",
        approval: "ask",
        authMode: "oauth",
        headers: {},
        oauthClient: { clientId: "client-1", clientSecret: "s3cret" },
      },
    });
    expect(parse(serverSchema, { ...input, auth: "oauth" })).toEqual({
      value: {
        name: "Docs",
        url: "https://mcp.example.com/mcp",
        approval: "ask",
        authMode: "oauth",
        headers: {},
      },
    });
  });

  test("a pre-registered client is one word, and a secret needs it", () => {
    expect(clientProblem("", "")).toBeUndefined();
    expect(clientProblem("", "secret")).toBe("Enter the client ID this secret belongs to.");
    expect(clientProblem("two words", "")).toBe(
      "The OAuth client ID must be one word of at most 512 characters."
    );
    expect(clientProblem("x".repeat(513), "")).toBe(
      "The OAuth client ID must be one word of at most 512 characters."
    );
    expect(clientProblem("client", "a\nb")).toBe(
      "The OAuth client secret must be one line of at most 512 characters."
    );
    expect(parse(clientSchema, { clientId: "client", clientSecret: "" })).toEqual({
      value: { clientId: "client" },
    });
    expect(parse(clientSchema, { clientId: " ", clientSecret: "" })).toEqual({ value: null });
    expect(parse(clientSchema, { clientId: "a b", clientSecret: "" })).toEqual({
      errors: { clientId: "The OAuth client ID must be one word of at most 512 characters." },
    });
  });

  test("the settings form sends only what changed", () => {
    const same = { name: "Docs", url: "https://mcp.example.com/mcp", approval: "ask" };
    expect(parse(settingsSchema(server), same)).toEqual({ value: {} });
    expect(
      parse(settingsSchema(server), {
        ...same,
        url: "https://other.example.com/mcp",
        approval: "auto",
      })
    ).toEqual({ value: { url: "https://other.example.com/mcp", approval: "auto" } });
  });
});

describe("tool modes", () => {
  test("a tool without its own mode follows the server's approval mode", () => {
    expect(modeFor(server, "search")).toBe("auto");
    expect(modeFor(server, "delete_page")).toBe("ask");
    expect(modeFor({ ...server, approval: "auto" }, "delete_page")).toBe("auto");
  });

  test("saving sends a mode for every listed tool and none for unlisted ones", () => {
    expect(modesBody(server, { delete_page: "off", gone: "auto" })).toEqual({
      search: "auto",
      delete_page: "off",
    });
  });

  test("the draft counts as changed only when a mode differs", () => {
    expect(modesChanged(server, {})).toBe(false);
    expect(modesChanged(server, { search: "auto" })).toBe(false);
    expect(modesChanged(server, { search: "ask" })).toBe(true);
  });

  test("tools read in title order", () => {
    expect(sortTools(server.tools ?? []).map((item) => item.name)).toEqual([
      "delete_page",
      "search",
    ]);
  });

  test("a test says what changed", () => {
    expect(testSummary(2, { added: [], removed: [], changed: [] })).toBe("Found 2 tools.");
    expect(testSummary(1, { added: ["a"], removed: ["b", "c"], changed: ["d"] })).toBe(
      "Found 1 tool. 1 added (a), 2 removed (b, c), 1 changed (d)."
    );
  });
});

describe("adding and testing a server", () => {
  let urls: string[] = [];

  function serve(test: Response): void {
    setTransport({
      server: () => "",
      token: () => "token",
      fetch: async (url) => {
        urls.push(url);
        if (url.endsWith("/test")) return test;
        return new Response(JSON.stringify({ data: { ...server, tools: null } }), {
          status: 201,
          headers: { "Content-Type": "application/json" },
        });
      },
    });
  }

  afterEach(() => {
    urls = [];
  });

  test("registers the server, then lists its tools", async () => {
    serve(
      new Response(
        JSON.stringify({
          data: { server, changes: { added: ["search"], removed: [], changed: [] } },
        }),
        { headers: { "Content-Type": "application/json" } }
      )
    );
    const added = await addServer({ name: "Docs", url: server.url ?? undefined, headers: {} });
    expect(urls).toEqual(["/api/tool-servers", `/api/tool-servers/${ID}/test`]);
    expect(added.test?.changes.added).toEqual(["search"]);
  });

  test("a server that signs in is not tested before its sign-in", async () => {
    setTransport({
      server: () => "",
      token: () => "token",
      fetch: async (url) => {
        urls.push(url);
        return new Response(JSON.stringify({ data: oauthServer }), {
          status: 201,
          headers: { "Content-Type": "application/json" },
        });
      },
    });
    const added = await addServer({
      name: "Docs",
      url: server.url ?? undefined,
      headers: {},
      authMode: "oauth",
    });
    expect(urls).toEqual(["/api/tool-servers"]);
    expect(added.test).toBeUndefined();
    expect(added.problem).toBeUndefined();
  });

  test("a failed test keeps the server and says why", async () => {
    serve(
      new Response(JSON.stringify({ error: "The tool server cannot be reached." }), {
        status: 502,
        headers: { "Content-Type": "application/json" },
      })
    );
    const added = await addServer({ name: "Docs", url: server.url ?? undefined, headers: {} });
    expect(added.server.id).toBe(ID);
    expect(added.problem).toBe("The tool server cannot be reached.");
  });
});

describe("the tool server screens", () => {
  const user: UserResponse = {
    id: "0195d6a4-1b2c-7d3e-8f40-123456789abc",
    username: "ada",
    displayName: "Ada",
    role: "user",
    disabled: false,
    createdAt: AT,
    updatedAt: AT,
  };

  function render(
    entry: string | { pathname: string; state: unknown },
    data: [readonly unknown[], unknown][]
  ) {
    const client = new QueryClient();
    setToken("token");
    client.setQueryData(["auth", "me", "token"], user);
    for (const [key, value] of data) client.setQueryData(key, value);
    return renderToString(
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={[entry]}>
          <Routes>
            <Route path={path.toolServers} element={<ToolServersPage />} />
            <Route path={path.toolServerNew} element={<ToolServerPage />} />
            <Route path={path.toolServer} element={<ToolServerPage />} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );
  }

  test("lists servers with their tools and what needs attention", () => {
    const html = render(path.toolServers, [
      [
        keys.toolServers,
        [
          { ...server, reviewRequired: true, approval: "auto" },
          { ...server, id: "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708101", name: "New", tools: null },
        ],
      ],
    ]);
    expect(html).toContain("2 tools · https://mcp.example.com/mcp");
    expect(html).toContain("Review changes");
    expect(html).toContain(">Auto<");
    expect(html).toContain("Not tested yet");
    expect(html).toContain(`href="${path.toolServerNew}"`);
  });

  test("the add screen asks for the address, authentication, and approval, Ask first", () => {
    const html = render(path.toolServerNew, []);
    expect(html).toContain("Add Tool Server");
    expect(html).toContain('name="url"');
    expect(checked(html, "authMode", "none")).toBe(true);
    expect(html).toContain("Token or headers");
    expect(html).toContain("Sign in");
    expect(html).not.toContain('name="bearer"');
    expect(checked(html, "approval", "ask")).toBe(true);
    expect(html).not.toContain("Auto lets the model run");
  });

  test("an OAuth server shows its sign-in state and the sign-in controls", () => {
    const html = render(`/settings/tools/${ID}`, [[keys.toolServer(ID), oauthServer]]);
    expect(checked(html, "authMode", "oauth")).toBe(true);
    expect(html).toContain("Needs sign-in");
    expect(html).toContain(">Sign in<");
    expect(html).not.toContain("Sign out");
    expect(html).toContain('name="clientId"');
    expect(html).not.toContain('name="bearer"');

    const signedIn = render(`/settings/tools/${ID}`, [
      [
        keys.toolServer(ID),
        {
          ...oauthServer,
          oauth: {
            state: "signed_in",
            clientId: "copilot-client",
            clientSecret: "set",
            expiresAt: AT,
          },
        },
      ],
    ]);
    expect(signedIn).toContain(">Signed in<");
    expect(signedIn).toContain("Sign in again");
    expect(signedIn).toContain("Sign out");
    expect(signedIn).toContain('value="copilot-client"');
    expect(signedIn).toContain("A secret is stored.");
  });

  test("the list names servers that need a sign-in", () => {
    const html = render(path.toolServers, [[keys.toolServers, [oauthServer]]]);
    expect(html).toContain("Needs sign-in");
  });

  test("a server shows its tools with their modes, settings, credentials, and delete", () => {
    const html = render(`/settings/tools/${ID}`, [[keys.toolServer(ID), server]]);
    expect(html).toContain("Search Docs");
    expect(html).toContain("Read-only");
    expect(checked(html, "mode-search", "auto")).toBe(true);
    expect(checked(html, "mode-delete_page", "ask")).toBe(true);
    expect(html).toContain("docs_&lt;tool&gt;");
    expect(html).toContain(">Stored<");
    expect(html).toContain("Replace credentials");
    expect(html).toContain("Delete Tool Server");
  });

  test("changed tools ask for a review, and the first test's outcome shows after adding", () => {
    const html = render(
      {
        pathname: `/settings/tools/${ID}`,
        state: { problem: "The tool server cannot be reached." },
      },
      [[keys.toolServer(ID), { ...server, reviewRequired: true, tools: null }]]
    );
    expect(html).toContain("Accept changes");
    expect(html).toContain("The connection test failed: The tool server cannot be reached.");
    expect(html).toContain("No tools listed yet.");
  });
});

describe("sign-in", () => {
  test("states read as badges, and only a usable sign-in counts", () => {
    expect(signInBadge("signed_in")).toEqual({ label: "Signed in", tone: "ok" });
    expect(signInBadge("expired").label).toBe("Expired: sign in again");
    expect(signInBadge("needs_sign_in").label).toBe("Needs sign-in");
    expect(needsSignIn(oauthServer)).toBe(true);
    expect(
      needsSignIn({
        ...oauthServer,
        oauth: { state: "signed_in", clientId: null, clientSecret: "missing", expiresAt: null },
      })
    ).toBe(false);
    expect(needsSignIn(server)).toBe(false);
  });

  test("opens the authorization page in place of the app", () => {
    const opened: string[] = [];
    openSignIn("https://auth.example.com/authorize?state=s", {
      assign: (url: string | URL) => {
        opened.push(String(url));
      },
    });
    expect(opened).toEqual(["https://auth.example.com/authorize?state=s"]);
  });

  test("the done page says how the sign-in ended and links only a server id", () => {
    expect(doneMessage("signed-in").title).toBe("Signed in");
    expect(doneMessage("denied").title).toBe("Sign-in cancelled");
    expect(doneMessage("expired").title).toBe("Sign-in expired");
    expect(doneMessage("failed").title).toBe("Sign-in failed");
    expect(doneMessage("<script>").title).toBe("Sign-in failed");
    expect(doneMessage(null).title).toBe("Sign-in failed");

    const page = (query: string) =>
      renderToString(
        <MemoryRouter initialEntries={[`${path.oauthDone}?${query}`]}>
          <Routes>
            <Route path={path.oauthDone} element={<SignInDonePage />} />
          </Routes>
        </MemoryRouter>
      );
    const done = page(`result=signed-in&server=${ID}`);
    expect(done).toContain("Signed in");
    expect(done).toContain("Open the tool server");
    const forged = page("result=signed-in&server=javascript:alert(1)");
    expect(forged).not.toContain("Open the tool server");
  });
});

const program: ToolServerResponse = {
  ...server,
  id: "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708102",
  name: "Files",
  slug: "files",
  transport: "stdio",
  url: null,
  credentials: "missing",
  authMode: "none",
  stdio: {
    command: "/usr/local/bin/files-mcp",
    args: ["--root", "/srv/data dir", "it's"],
    envNames: ["API_KEY"],
    env: "set",
    cwd: "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708102",
  },
};

describe("stdio programs", () => {
  const input: ProgramInput = { command: "/usr/local/bin/files-mcp", args: "", rows: [], cwd: "" };

  test("the command line shows every argument, quoted only where needed", () => {
    expect(quote("--root")).toBe("--root");
    expect(quote("/srv/data dir")).toBe("'/srv/data dir'");
    expect(quote("it's")).toBe("'it'\\''s'");
    expect(quote("")).toBe("''");
    expect(quote("$HOME;rm")).toBe("'$HOME;rm'");
    expect(commandLine("/bin/x", ["a b", "c"])).toBe("/bin/x 'a b' c");
    const long = "x".repeat(3000);
    expect(commandLine("/bin/x", [long])).toContain(long);
    expect(location(program)).toBe("/usr/local/bin/files-mcp --root '/srv/data dir' 'it'\\''s'");
    expect(location(server)).toBe("https://mcp.example.com/mcp");
  });

  test("one argument per line, kept exactly, blank lines left out", () => {
    expect(parseArgs("--root\n /srv \r\n\n--flag")).toEqual(["--root", " /srv ", "--flag"]);
    expect(parseArgs("")).toEqual([]);
  });

  test("the program follows the server's rules", () => {
    const field = (change: Partial<ProgramInput>) => programProblem({ ...input, ...change })?.field;
    expect(programProblem(input)).toBeUndefined();
    expect(field({ command: "" })).toBe("command");
    expect(field({ command: "npx" })).toBe("command");
    expect(programProblem({ ...input, command: "npx" })?.message).toContain("absolute path");
    expect(field({ command: "/bin/a\nb" })).toBe("command");
    expect(field({ args: Array.from({ length: 65 }, () => "a").join("\n") })).toBe("args");
    expect(field({ rows: [row("1ABC", "v")] })).toBe("env");
    expect(field({ rows: [row("A-B", "v")] })).toBe("env");
    expect(field({ rows: [row("KEY", "a"), row(" KEY ", "b")] })).toBe("env");
    expect(field({ rows: [row("KEY", " spaced value ")] })).toBeUndefined();
    for (const cwd of ["../etc", "/etc", "a/../b", "a//b", "a b"]) {
      expect(field({ cwd })).toBe("cwd");
    }
    expect(field({ cwd: "team/files_1.0" })).toBeUndefined();
  });

  test("adding a stdio server sends its program, never an address or credentials", () => {
    const parsed = parse(serverSchema, {
      name: "Files",
      slug: "",
      transport: "stdio",
      url: "",
      approval: "ask",
      auth: "headers",
      bearer: "tok",
      rows: [row("X-Team", "a")],
      clientId: "",
      clientSecret: "",
      command: " /usr/local/bin/files-mcp ",
      args: "--root\n/srv/data dir",
      envRows: [row("API_KEY", "sk-1"), row("", "")],
      cwd: "",
    });
    expect(parsed).toEqual({
      value: {
        name: "Files",
        transport: "stdio",
        approval: "ask",
        headers: {},
        stdio: {
          command: "/usr/local/bin/files-mcp",
          args: ["--root", "/srv/data dir"],
          env: { API_KEY: "sk-1" },
        },
      },
    });
    const relative = parse(serverSchema, {
      name: "Files",
      slug: "",
      transport: "stdio",
      url: "",
      approval: "ask",
      auth: "none",
      bearer: "",
      rows: [],
      clientId: "",
      clientSecret: "",
      command: "npx",
    });
    expect(Object.keys(relative.errors ?? {})).toEqual(["command"]);
  });

  test("a program change sends only what changed, and the variables only when entered", () => {
    const same = {
      command: "/usr/local/bin/files-mcp",
      args: "--root\n/srv/data dir\nit's",
      rows: [],
      cwd: "",
    };
    expect(programPatch(program, same, false)).toEqual({});
    expect(programPatch(program, same, true)).toEqual({ env: {} });
    expect(
      programPatch(program, { ...same, args: "--write", rows: [row("MODE", "rw")] }, false)
    ).toEqual({ args: ["--write"], env: { MODE: "rw" } });
    expect(programPatch(program, { ...same, cwd: "shared" }, false)).toEqual({ cwd: "shared" });
    expect(parse(programSchema(program, false), { ...same, command: "rel" }).errors).toEqual({
      command: expect.stringContaining("absolute path"),
    });
  });

  test("a stdio server has no address in its settings", () => {
    expect(parse(settingsSchema(program), { name: "Files 2", approval: "ask" })).toEqual({
      value: { name: "Files 2" },
    });
  });
});

describe("the stdio screens", () => {
  const admin: UserResponse = {
    id: "0195d6a4-1b2c-7d3e-8f40-123456789abd",
    username: "root",
    displayName: "Root",
    role: "administrator",
    disabled: false,
    createdAt: AT,
    updatedAt: AT,
  };

  function render(entry: string, data: [readonly unknown[], unknown][]) {
    const client = new QueryClient();
    setToken("token");
    client.setQueryData(["auth", "me", "token"], admin);
    for (const [key, value] of data) client.setQueryData(key, value);
    return renderToString(
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={[entry]}>
          <Routes>
            <Route path={path.toolServers} element={<ToolServersPage />} />
            <Route path={path.toolServerNew} element={<ToolServerPage />} />
            <Route path={path.toolServer} element={<ToolServerPage />} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );
  }

  test("the add screen offers a program only when the server allows the caller stdio", () => {
    const off = render(path.toolServerNew, [[keys.capabilities, { stdioServers: false }]]);
    expect(off).not.toContain('name="transport"');
    const on = render(path.toolServerNew, [[keys.capabilities, { stdioServers: true }]]);
    expect(on).toContain('name="transport"');
    expect(checked(on, "transport", "http")).toBe(true);
    expect(on).toContain("Program on this server");
  });

  test("the list names a program by its command line", () => {
    const html = render(path.toolServers, [[keys.toolServers, [program]]]);
    expect(html).toContain("/usr/local/bin/files-mcp --root");
    expect(html).toContain(">Program<");
  });

  test("a stdio server shows its whole program, and the form only to an administrator", () => {
    const long = "x".repeat(2000);
    const base = program.stdio;
    if (!base) throw new Error("a stdio server");
    const stored = { ...program, stdio: { ...base, args: ["--root", long] } };
    const html = render(`/settings/tools/${program.id}`, [
      [keys.toolServer(program.id), stored],
      [keys.capabilities, { stdioServers: true }],
    ]);
    expect(html).toContain(long);
    expect(html).toContain("program-line");
    expect(html).toContain("Environment: only API_KEY.");
    expect(html).toContain("Save program");
    expect(html).toContain("Remove the stored variables");
    expect(html).not.toContain('name="authMode"');
    expect(html).not.toContain('name="url"');

    const viewer = render(`/settings/tools/${program.id}`, [
      [keys.toolServer(program.id), stored],
      [keys.capabilities, { stdioServers: false }],
    ]);
    expect(viewer).toContain(long);
    expect(viewer).not.toContain("Save program");
    expect(viewer).toContain("Only an administrator can change the program");
  });

  test("a review shows the program whose tools it accepts", () => {
    const html = render(`/settings/tools/${program.id}`, [
      [keys.toolServer(program.id), { ...program, reviewRequired: true }],
      [keys.capabilities, { stdioServers: false }],
    ]);
    expect(html).toContain("Accept changes");
    expect(html).toContain("You accept the tools of this program:");
    expect(html).toContain("--root &#x27;/srv/data dir&#x27;");
  });
});
