// The tool server forms of Settings: credentials (a bearer token and custom headers, checked with
// the server's rules before sending), a pre-registered OAuth client, the server's own settings,
// and adding a server (remote, or a stdio program for administrators: `stdio.ts`).

import type {
  ApprovalMode,
  AuthMode,
  CreateToolServerRequest,
  Credentials,
  OAuthClientInput,
  ToolServerResponse,
  ToolTestResponse,
  UpdateToolServerRequest,
} from "@copilot/contracts";
import { describeError } from "@web/api/errors";
import { createToolServer, testToolServer } from "@web/api/routes/tool-servers";
import { isHttp } from "@web/forms";
import { NAME_MAX_CHARS } from "@web/routes/settings/providers/presets";
import { environment, parseArgs, programProblem } from "@web/routes/settings/tool-server/stdio";
import { z } from "zod";

/** One custom header row of the form; `id` keeps React's keys stable while rows are removed. */
export type HeaderRow = { id: number; name: string; value: string };

/** Headers the MCP client sets itself (compared in lowercase), plus every `mcp-*` name. */
const RESERVED = new Set([
  "accept",
  "connection",
  "content-length",
  "content-type",
  "host",
  "last-event-id",
  "mcp-protocol-version",
  "transfer-encoding",
]);

/** An HTTP token (RFC 9110). */
const TOKEN = /^[A-Za-z0-9!#$%&'*+.^_`|~-]+$/;

/** Control characters other than tab (Rust's `char::is_control`, which the server uses). */
const CONTROL = /(?!\t)\p{Cc}/u;

/** Longest slug the server accepts. */
export const SLUG_MAX_CHARS = 12;

/** Longest OAuth client id or secret the server accepts. */
export const OAUTH_CLIENT_MAX_CHARS = 512;

/** Why one header is refused, with the server's message; `seen` holds the lowercase names before it. */
function headerProblem(
  name: string,
  value: string,
  bearer: string,
  seen: ReadonlySet<string>
): string | undefined {
  const lower = name.toLowerCase();
  if (!TOKEN.test(name)) {
    return "Header names may contain only letters, digits, and !#$%&'*+-.^_`|~.";
  }
  if (RESERVED.has(lower) || lower.startsWith("mcp-"))
    return `The ${name} header is set by Copilot.`;
  if (lower === "authorization" && bearer !== "") {
    return "Set either a bearer token or a custom Authorization header, not both.";
  }
  if (seen.has(lower)) return `The ${name} header is listed twice.`;
  if (value === "") return `Enter a value for the ${name} header.`;
  if (CONTROL.test(value)) return `The ${name} header must be a single line.`;
  return undefined;
}

/**
 * Why the header rows and bearer token are refused, with the server's messages, or `undefined`
 * when they are fine. Rows with neither a name nor a value are ignored.
 */
export function credentialsProblem(
  bearer: string,
  rows: readonly HeaderRow[]
): { field: "bearer" | "headers"; message: string } | undefined {
  const token = bearer.trim();
  const seen = new Set<string>();
  for (const row of rows) {
    const name = row.name.trim();
    const value = row.value.trim();
    if (name === "" && value === "") continue;
    const message = headerProblem(name, value, token, seen);
    if (message) return { field: "headers", message };
    seen.add(name.toLowerCase());
  }
  if (/[\s\p{Cc}]/u.test(token)) {
    return { field: "bearer", message: "The bearer token must not contain spaces or line breaks." };
  }
  return undefined;
}

/** The message when `Token or headers` has neither. */
export const NO_CREDENTIALS = "Enter a bearer token or at least one header.";

/**
 * Why a pre-registered OAuth client is refused, with the server's messages, or `undefined`. A
 * blank id means no client; a secret needs an id.
 */
export function clientProblem(clientId: string, clientSecret: string): string | undefined {
  const id = clientId.trim();
  const secret = clientSecret.trim();
  if (id === "") return secret === "" ? undefined : "Enter the client ID this secret belongs to.";
  if (Array.from(id).length > OAUTH_CLIENT_MAX_CHARS || /[\s\p{Cc}]/u.test(id)) {
    return `The OAuth client ID must be one word of at most ${OAUTH_CLIENT_MAX_CHARS} characters.`;
  }
  if (Array.from(secret).length > OAUTH_CLIENT_MAX_CHARS || /\p{Cc}/u.test(secret)) {
    return `The OAuth client secret must be one line of at most ${OAUTH_CLIENT_MAX_CHARS} characters.`;
  }
  return undefined;
}

/** The client the fields make, or `undefined` for a blank id. A blank secret is left out. */
export function oauthClient(clientId: string, clientSecret: string): OAuthClientInput | undefined {
  const id = clientId.trim();
  if (id === "") return undefined;
  const secret = clientSecret.trim();
  return secret === "" ? { clientId: id } : { clientId: id, clientSecret: secret };
}

/**
 * The OAuth client form of an OAuth server; parses to the `oauthClient` of a `PATCH` (`null`
 * for a blank id, which removes the stored client).
 */
export const clientSchema = z
  .object({ clientId: z.string(), clientSecret: z.string() })
  .superRefine((input, context) => {
    const problem = clientProblem(input.clientId, input.clientSecret);
    if (problem) context.addIssue({ code: "custom", path: ["clientId"], message: problem });
  })
  .transform((input) => oauthClient(input.clientId, input.clientSecret) ?? null);

/** The code form of a sign-in that came back: the code the browser showed, as typed. */
export const confirmSchema = z.object({
  code: z.string().trim().min(1, "Enter the code the browser showed."),
});

/** The credentials the rows and token make: blank rows and a blank token are left out. */
export function credentials(bearer: string, rows: readonly HeaderRow[]): Credentials {
  const headers: Record<string, string> = {};
  for (const row of rows) {
    const name = row.name.trim();
    if (name !== "") headers[name] = row.value.trim();
  }
  const token = bearer.trim();
  return token === "" ? { headers } : { bearer: token, headers };
}

/** The Credentials form (`bearer` and `rows`); parses to the `credentials` of a `PATCH`. */
export const credentialsSchema = z
  .object({ bearer: z.string(), rows: z.array(z.custom<HeaderRow>()) })
  .superRefine((input, context) => {
    const problem = credentialsProblem(input.bearer, input.rows);
    if (problem)
      context.addIssue({ code: "custom", path: [problem.field], message: problem.message });
  })
  .transform((input) => credentials(input.bearer, input.rows));

const name = z
  .string()
  .trim()
  .min(1, "Enter a name.")
  .refine((value) => Array.from(value).length <= NAME_MAX_CHARS, "Use at most 100 characters.");

const url = z.string().trim().refine(isHttp, "Enter a full http:// or https:// address.");

const approval = z.enum(["ask", "auto"]);

const authMode = z.enum(["none", "headers", "oauth"]);

/**
 * The add-server form; parses to the `POST /tool-servers` body. Only the chosen authentication's
 * fields are sent: a token or headers for `headers` (one of them needed), a client for `oauth`.
 * A `stdio` server sends its program instead of an address and credentials.
 */
export const serverSchema = z
  .object({
    name,
    slug: z
      .string()
      .trim()
      .refine(
        (value) => value === "" || new RegExp(`^[a-z0-9]{1,${SLUG_MAX_CHARS}}$`).test(value),
        `The slug must be 1 to ${SLUG_MAX_CHARS} lowercase letters or digits.`
      ),
    transport: z.enum(["http", "stdio"]).default("http"),
    url: z.string().trim(),
    approval,
    auth: authMode,
    bearer: z.string(),
    rows: z.array(z.custom<HeaderRow>()),
    clientId: z.string(),
    clientSecret: z.string(),
    command: z.string().default(""),
    args: z.string().default(""),
    envRows: z.array(z.custom<HeaderRow>()).default([]),
    cwd: z.string().default(""),
  })
  .superRefine((input, context) => {
    if (input.transport === "stdio") {
      const problem = programProblem({
        command: input.command,
        args: input.args,
        rows: input.envRows,
        cwd: input.cwd,
      });
      if (problem) {
        context.addIssue({ code: "custom", path: [problem.field], message: problem.message });
      }
      return;
    }
    if (!isHttp(input.url)) {
      context.addIssue({
        code: "custom",
        path: ["url"],
        message: "Enter a full http:// or https:// address.",
      });
    }
    if (input.auth === "headers") {
      const problem = credentialsProblem(input.bearer, input.rows);
      if (problem) {
        context.addIssue({ code: "custom", path: [problem.field], message: problem.message });
      } else if (isEmpty(credentials(input.bearer, input.rows))) {
        context.addIssue({ code: "custom", path: ["bearer"], message: NO_CREDENTIALS });
      }
    }
    if (input.auth === "oauth") {
      const problem = clientProblem(input.clientId, input.clientSecret);
      if (problem) context.addIssue({ code: "custom", path: ["clientId"], message: problem });
    }
  })
  .transform((input): CreateToolServerRequest => {
    if (input.transport === "stdio") {
      const cwd = input.cwd.trim();
      return {
        name: input.name,
        transport: "stdio",
        approval: input.approval,
        headers: {},
        stdio: {
          command: input.command.trim(),
          args: parseArgs(input.args),
          env: environment(input.envRows),
          ...(cwd ? { cwd } : {}),
        },
        ...(input.slug ? { slug: input.slug } : {}),
      };
    }
    const { bearer, headers } =
      input.auth === "headers" ? credentials(input.bearer, input.rows) : { headers: {} };
    const client =
      input.auth === "oauth" ? oauthClient(input.clientId, input.clientSecret) : undefined;
    return {
      name: input.name,
      url: input.url,
      approval: input.approval,
      authMode: input.auth as AuthMode,
      headers,
      ...(input.slug ? { slug: input.slug } : {}),
      ...(bearer ? { bearer } : {}),
      ...(client ? { oauthClient: client } : {}),
    };
  });

/** Credentials without a token or a header. */
export function isEmpty(value: Credentials): boolean {
  return value.bearer === undefined && Object.keys(value.headers ?? {}).length === 0;
}

/**
 * The Server form of an existing server; parses to a `PATCH` body with only what changed. A
 * stdio server has no address (its program has its own form).
 */
export function settingsSchema(server: ToolServerResponse) {
  const address = server.transport === "stdio" ? z.string().optional() : url;
  return z.object({ name, url: address, approval }).transform((input): UpdateToolServerRequest => {
    const body: UpdateToolServerRequest = {};
    if (input.name !== server.name) body.name = input.name;
    if (server.transport !== "stdio" && input.url !== server.url) body.url = input.url;
    if (input.approval !== server.approval) body.approval = input.approval as ApprovalMode;
    return body;
  });
}

/** A new server and its first test: the tools it listed, or why listing failed. */
export type Added = {
  server: ToolServerResponse;
  test?: ToolTestResponse;
  /** Why the test failed (unreachable, rejected credentials). */
  problem?: string;
};

/** What the add-server screen hands the server's page (`location.state`). */
export type AddedState = Pick<Added, "test" | "problem">;

/**
 * Registers a server, then tests it (connects and lists its tools; a stdio server's program
 * starts for the test and ends with it). A failed test keeps the server, so the user can fix the
 * address, credentials, or program. A server that signs in with OAuth is not tested: it has no
 * sign-in yet, and its page offers one.
 */
export async function addServer(body: CreateToolServerRequest): Promise<Added> {
  const server = await createToolServer(body);
  if (server.authMode === "oauth") return { server };
  try {
    return { server, test: await testToolServer(server.id) };
  } catch (error) {
    return { server, problem: describeError(error).message };
  }
}
