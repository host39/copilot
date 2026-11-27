// stdio tool servers: a program on the Copilot server that speaks MCP over its stdin and stdout.
// Only administrators add and change them, and only while the server allows it
// (`GET /capabilities`). The rules mirror the server's, and the program is always shown whole:
// the command and every argument, never truncated, before it is saved and wherever it is
// reviewed.

import type { StdioPatch, StdioProgram, ToolServerResponse } from "@copilot/contracts";
import type { HeaderRow } from "@web/routes/settings/tool-server/forms";
import { z } from "zod";

/** The server's limits (`STDIO_*` in core's constants). */
export const COMMAND_MAX_CHARS = 1024;
export const ARGS_MAX = 64;
export const ARG_MAX_CHARS = 4096;
export const ENV_MAX = 64;
export const ENV_NAME_MAX_CHARS = 128;
export const ENV_VALUE_MAX_CHARS = 8192;
export const CWD_MAX_CHARS = 255;

/** Control characters (Rust's `char::is_control`, which the server uses for the path). */
const CONTROL = /\p{Cc}/u;

/** An argument that reads the same without quotes. */
const PLAIN = /^[A-Za-z0-9_@%+=:,./-]+$/;

/** An environment variable name: letters, digits, and `_`, not starting with a digit. */
const ENV_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;

/** One folder name of the working directory. */
const FOLDER = /^[A-Za-z0-9._-]+$/;

/** What the program form holds. `rows` are the environment variables (name and value). */
export type ProgramInput = {
  command: string;
  /** One argument per line. */
  args: string;
  rows: readonly HeaderRow[];
  cwd: string;
};

/**
 * One argument as a shell would need it written, so the command line reads unambiguously: plain
 * words as they are, anything else in single quotes. Display only; the server runs the program
 * with these arguments directly, without a shell.
 */
export function quote(arg: string): string {
  if (PLAIN.test(arg)) return arg;
  return `'${arg.replaceAll("'", "'\\''")}'`;
}

/** The whole command line, every argument, never shortened. */
export function commandLine(command: string, args: readonly string[]): string {
  return [command, ...args].map(quote).join(" ");
}

/** The command line of a stored program. */
export function programLine(program: Pick<StdioProgram, "command" | "args">): string {
  return commandLine(program.command, program.args);
}

/** Where a server is: its URL, or the command line of its program. */
export function location(server: ToolServerResponse): string {
  return server.url ?? (server.stdio ? programLine(server.stdio) : "");
}

/** The arguments of the text box: one per line, blank lines left out, each kept exactly. */
export function parseArgs(text: string): string[] {
  return text.split(/\r?\n/).filter((line) => line !== "");
}

/** The environment the rows make: rows with neither a name nor a value are left out. */
export function environment(rows: readonly HeaderRow[]): Record<string, string> {
  const env: Record<string, string> = {};
  for (const row of rows) {
    const name = row.name.trim();
    if (name !== "") env[name] = row.value;
  }
  return env;
}

type Problem = { field: "command" | "args" | "env" | "cwd"; message: string };

/** Why the program is refused, with the server's messages, or `undefined` when it is fine. */
export function programProblem(input: ProgramInput): Problem | undefined {
  return (
    commandProblem(input.command) ??
    argsProblem(parseArgs(input.args)) ??
    envProblem(input.rows) ??
    cwdProblem(input.cwd)
  );
}

function commandProblem(text: string): Problem | undefined {
  const command = text.trim();
  if (command === "") return { field: "command", message: "Enter the program's path." };
  if (!command.startsWith("/")) {
    return {
      field: "command",
      message:
        "Enter the program's absolute path (for example /usr/local/bin/my-server): it starts without PATH, so names are not looked up.",
    };
  }
  if (Array.from(command).length > COMMAND_MAX_CHARS || CONTROL.test(command)) {
    return {
      field: "command",
      message: `The program's path must be one line of at most ${COMMAND_MAX_CHARS} characters.`,
    };
  }
  return undefined;
}

function argsProblem(args: readonly string[]): Problem | undefined {
  if (args.length > ARGS_MAX) {
    return { field: "args", message: `A stdio server takes at most ${ARGS_MAX} arguments.` };
  }
  if (args.some((arg) => arg.includes("\0") || Array.from(arg).length > ARG_MAX_CHARS)) {
    return {
      field: "args",
      message: `Each argument must be at most ${ARG_MAX_CHARS} characters, without NUL characters.`,
    };
  }
  return undefined;
}

function variableProblem(name: string, value: string): string | undefined {
  if (!ENV_NAME.test(name) || name.length > ENV_NAME_MAX_CHARS) {
    return `Environment variable names are letters, digits, and _ (not starting with a digit), at most ${ENV_NAME_MAX_CHARS} characters.`;
  }
  if (value.includes("\0") || Array.from(value).length > ENV_VALUE_MAX_CHARS) {
    return `The value of ${name} must be at most ${ENV_VALUE_MAX_CHARS} characters, without NUL characters.`;
  }
  return undefined;
}

function envProblem(rows: readonly HeaderRow[]): Problem | undefined {
  const used = rows.filter((row) => row.name.trim() !== "" || row.value !== "");
  if (used.length > ENV_MAX) {
    return {
      field: "env",
      message: `A stdio server takes at most ${ENV_MAX} environment variables.`,
    };
  }
  const seen = new Set<string>();
  for (const row of used) {
    const name = row.name.trim();
    const message = variableProblem(name, row.value);
    if (message) return { field: "env", message };
    if (seen.has(name)) return { field: "env", message: `${name} is listed twice.` };
    seen.add(name);
  }
  return undefined;
}

function cwdProblem(text: string): Problem | undefined {
  const cwd = text.trim();
  if (cwd === "" || (cwd.length <= CWD_MAX_CHARS && cwd.split("/").every(isFolder))) {
    return undefined;
  }
  return {
    field: "cwd",
    message: `The working directory is a folder inside the stdio base directory: names of letters, digits, ., _, and - joined by /, at most ${CWD_MAX_CHARS} characters, without . or .. parts.`,
  };
}

function isFolder(part: string): boolean {
  return part !== "." && part !== ".." && FOLDER.test(part);
}

/**
 * The `stdio` of a `PATCH` for `server`: only what changed. Variables are sent only when the
 * form has some (they replace every stored one) or `clearEnv` is set; a blank directory goes back
 * to the server's own.
 */
export function programPatch(
  server: ToolServerResponse,
  input: ProgramInput,
  clearEnv: boolean
): StdioPatch {
  const stored = server.stdio;
  const patch: StdioPatch = {};
  const command = input.command.trim();
  const args = parseArgs(input.args);
  if (command !== stored?.command) patch.command = command;
  if (JSON.stringify(args) !== JSON.stringify(stored?.args ?? [])) patch.args = args;
  const env = environment(input.rows);
  if (Object.keys(env).length > 0 || clearEnv) patch.env = env;
  const cwd = input.cwd.trim();
  if (cwd !== stored?.cwd && !(cwd === "" && stored?.cwd === server.id)) patch.cwd = cwd;
  return patch;
}

/** Field errors of the program fields, the form's own or the server's (`stdio.command`). */
export function programErrors(
  errors: Partial<Record<string, string>>
): Partial<Record<"command" | "args" | "env" | "cwd", string>> {
  const pick = (field: string) => errors[field] ?? errors[`stdio.${field}`];
  return {
    command: pick("command"),
    args: pick("args"),
    env: pick("env"),
    cwd: pick("cwd"),
  };
}

/**
 * The Program form of a stdio server; parses to the `stdio` of a `PATCH` with only what changed
 * (`clearEnv` removes the stored variables when none are entered).
 */
export function programSchema(server: ToolServerResponse, clearEnv: boolean) {
  return z
    .object({
      command: z.string(),
      args: z.string(),
      rows: z.array(z.custom<HeaderRow>()),
      cwd: z.string(),
    })
    .superRefine((input, context) => {
      const problem = programProblem(input);
      if (problem) {
        context.addIssue({ code: "custom", path: [problem.field], message: problem.message });
      }
    })
    .transform((input) => programPatch(server, input, clearEnv));
}

/** The variable names a saved `patch` leaves the server with. */
export function envNamesAfter(server: ToolServerResponse, patch: StdioPatch): string[] {
  return patch.env ? Object.keys(patch.env) : (server.stdio?.envNames ?? []);
}
