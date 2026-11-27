// The tool server forms of Settings: credentials (a bearer token and custom headers, checked with
// the server's rules before sending), the server's own settings, and adding a server.

import type {
  ApprovalMode,
  CreateToolServerRequest,
  Credentials,
  ToolServerResponse,
  ToolTestResponse,
  UpdateToolServerRequest,
} from "@copilot/contracts";
import { describeError } from "@web/api/errors";
import { createToolServer, testToolServer } from "@web/api/routes/tool-servers";
import { NAME_MAX_CHARS } from "@web/routes/settings/providers/presets";
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

function isHttp(value: string): boolean {
  try {
    const url = new URL(value);
    return url.protocol === "http:" || url.protocol === "https:";
  } catch {
    return false;
  }
}

const name = z
  .string()
  .trim()
  .min(1, "Enter a name.")
  .refine((value) => Array.from(value).length <= NAME_MAX_CHARS, "Use at most 100 characters.");

const url = z.string().trim().refine(isHttp, "Enter a full http:// or https:// address.");

const approval = z.enum(["ask", "auto"]);

/** The add-server form; parses to the `POST /tool-servers` body. */
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
    url,
    approval,
    bearer: z.string(),
    rows: z.array(z.custom<HeaderRow>()),
  })
  .superRefine((input, context) => {
    const problem = credentialsProblem(input.bearer, input.rows);
    if (problem)
      context.addIssue({ code: "custom", path: [problem.field], message: problem.message });
  })
  .transform((input): CreateToolServerRequest => {
    const { bearer, headers } = credentials(input.bearer, input.rows);
    return {
      name: input.name,
      url: input.url,
      approval: input.approval,
      headers,
      ...(input.slug ? { slug: input.slug } : {}),
      ...(bearer ? { bearer } : {}),
    };
  });

/** The Server form of an existing server; parses to a `PATCH` body with only what changed. */
export function settingsSchema(server: ToolServerResponse) {
  return z.object({ name, url, approval }).transform((input): UpdateToolServerRequest => {
    const body: UpdateToolServerRequest = {};
    if (input.name !== server.name) body.name = input.name;
    if (input.url !== server.url) body.url = input.url;
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
 * Registers a server, then tests it (connects and lists its tools). A failed test keeps the
 * server, so the user can fix the address or credentials.
 */
export async function addServer(body: CreateToolServerRequest): Promise<Added> {
  const server = await createToolServer(body);
  try {
    return { server, test: await testToolServer(server.id) };
  } catch (error) {
    return { server, problem: describeError(error).message };
  }
}
