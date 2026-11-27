// The schedule form's rules: presets that write a cron rule (and read one back), the device's time
// zone, the checks core makes on a schedule, the request bodies, what the Scheduled list says
// about a schedule, and the tools a run would use. Pure functions, so tests check every rule
// without a browser.

import type {
  ChatResponse,
  ChatSummaryResponse,
  CreateScheduleRequest,
  MessagePartDto,
  MessageResponse,
  ModelResponse,
  OnAsk,
  ProjectResponse,
  ReasoningLevel,
  ScheduleResponse,
  ToolMode,
  ToolServerResponse,
  UpdateScheduleRequest,
} from "@copilot/contracts";
import type { BadgeTone } from "@web/components/Badge/Badge";
import { messageText } from "@web/components/Chat/messages";
import { startModel } from "@web/components/ModelPicker/models";
import { modeFor } from "@web/routes/settings/tool-server/modes";
import { z } from "zod";

type ChatToolCall = Extract<MessagePartDto, { type: "toolCall" }>;

/** Core's `NAME_MAX_CHARS`, which bounds a schedule's name. */
export const NAME_MAX_CHARS = 100;

export type Preset = "hourly" | "daily" | "weekdays" | "weekly" | "monthly" | "custom";

export const PRESETS: readonly { value: Preset; label: string }[] = [
  { value: "hourly", label: "Every hour" },
  { value: "daily", label: "Every day at" },
  { value: "weekdays", label: "Weekdays at" },
  { value: "weekly", label: "Weekly on" },
  { value: "monthly", label: "Monthly on day" },
  { value: "custom", label: "Custom cron" },
];

/** Day names by cron number (0 is Sunday). */
export const WEEKDAYS = [
  "Sunday",
  "Monday",
  "Tuesday",
  "Wednesday",
  "Thursday",
  "Friday",
  "Saturday",
] as const;

/**
 * The rule the form edits. Every preset's value stays while the user switches presets, so going
 * back keeps what they chose.
 */
export type Rule = {
  preset: Preset;
  /** `HH:MM` (24 hours), for every preset but hourly and custom. */
  time: string;
  /** The minute past the hour, for hourly. */
  minute: number;
  /** 0 (Sunday) to 6, for weekly. */
  weekday: number;
  /** 1 to 31, for monthly. */
  day: number;
  /** The rule as typed, for custom. */
  cron: string;
};

export const DEFAULT_RULE: Rule = {
  preset: "daily",
  time: "09:00",
  minute: 0,
  weekday: 1,
  day: 1,
  cron: "0 9 * * *",
};

const TIME = /^([01]\d|2[0-3]):([0-5]\d)$/;

function clock(time: string): { hour: number; minute: number } | null {
  const match = TIME.exec(time);
  if (!match) return null;
  return { hour: Number(match[1]), minute: Number(match[2]) };
}

/** A typed cron rule with single spaces between its fields. */
export function tidyCron(cron: string): string {
  return cron.trim().split(/\s+/).filter(Boolean).join(" ");
}

/** The cron rule `rule` stands for (custom: as typed, tidied). */
export function toCron(rule: Rule): string {
  if (rule.preset === "custom") return tidyCron(rule.cron);
  if (rule.preset === "hourly") return `${rule.minute} * * * *`;
  const { hour, minute } = clock(rule.time) ?? { hour: 9, minute: 0 };
  switch (rule.preset) {
    case "daily":
      return `${minute} ${hour} * * *`;
    case "weekdays":
      return `${minute} ${hour} * * 1-5`;
    case "weekly":
      return `${minute} ${hour} * * ${rule.weekday}`;
    case "monthly":
      return `${minute} ${hour} ${rule.day} * *`;
  }
}

function number(field: string | undefined, low: number, high: number): number | null {
  if (field === undefined || !/^\d{1,2}$/.test(field)) return null;
  const value = Number(field);
  return value >= low && value <= high ? value : null;
}

const pad = (value: number) => value.toString().padStart(2, "0");

/** The preset that writes `day` and `weekday` at a fixed time, or `null` for none. */
function dated(
  base: Rule,
  dayField: string | undefined,
  weekdayField: string | undefined
): Rule | null {
  if (dayField === "*") {
    if (weekdayField === "*") return { ...base, preset: "daily" };
    if (weekdayField === "1-5") return { ...base, preset: "weekdays" };
    const weekday = number(weekdayField, 0, 7);
    return weekday === null ? null : { ...base, preset: "weekly", weekday: weekday % 7 };
  }
  const day = number(dayField, 1, 31);
  return day !== null && weekdayField === "*" ? { ...base, preset: "monthly", day } : null;
}

/**
 * The form's rule for a saved cron rule: the preset that writes exactly this rule, else custom.
 * `7` is Sunday too in cron, so `weekly` reads it as `0`.
 */
export function readRule(cron: string): Rule {
  const tidy = tidyCron(cron);
  const custom: Rule = { ...DEFAULT_RULE, preset: "custom", cron: tidy };
  const fields = tidy.split(" ");
  if (fields.length !== 5) return custom;
  const [minuteField, hourField, dayField, monthField, weekdayField] = fields;
  const minute = number(minuteField, 0, 59);
  if (minute === null || monthField !== "*") return custom;
  if (hourField === "*") {
    const hourly = dayField === "*" && weekdayField === "*";
    return hourly ? { ...DEFAULT_RULE, preset: "hourly", minute, cron: tidy } : custom;
  }
  const hour = number(hourField, 0, 23);
  if (hour === null) return custom;
  const base = { ...DEFAULT_RULE, time: `${pad(hour)}:${pad(minute)}`, minute, cron: tidy };
  return dated(base, dayField, weekdayField) ?? custom;
}

/** What is wrong with the rule before the server sees it, by field (`cron`, `time`, `day`). */
export function ruleErrors(rule: Rule): Record<string, string> {
  if (rule.preset === "custom") {
    const fields = tidyCron(rule.cron);
    if (!fields) return { cron: "Enter a cron rule." };
    if (fields.split(" ").length !== 5) {
      return {
        cron: "A cron rule has 5 fields: minute, hour, day of month, month, and day of week.",
      };
    }
    return {};
  }
  if (rule.preset === "hourly") {
    return Number.isInteger(rule.minute) && rule.minute >= 0 && rule.minute <= 59
      ? {}
      : { time: "Choose a minute from 0 to 59." };
  }
  const errors: Record<string, string> = clock(rule.time)
    ? {}
    : { time: "Enter a time, for example 09:00." };
  if (
    rule.preset === "monthly" &&
    !(Number.isInteger(rule.day) && rule.day >= 1 && rule.day <= 31)
  ) {
    errors.day = "Choose a day from 1 to 31.";
  }
  return errors;
}

/** The device's time zone (`Intl`), the form's default; `UTC` where the browser names none. */
export function deviceZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

/** The time zones to choose from: the browser's list, plus `UTC` and `extra` (the saved zone). */
export function zoneList(...extra: string[]): string[] {
  let known: string[] = [];
  try {
    known = Intl.supportedValuesOf?.("timeZone") ?? [];
  } catch {
    known = [];
  }
  const all = new Set([...known, "UTC", ...extra.filter(Boolean)]);
  return [...all].sort((a, b) => a.localeCompare(b, "en"));
}

const name = z
  .string()
  .trim()
  .min(1, "Schedule name is required.")
  .refine((value) => !/\p{Cc}/u.test(value), "Schedule name must be a single line.")
  .refine(
    (value) => [...value].length <= NAME_MAX_CHARS,
    `Schedule name must be at most ${NAME_MAX_CHARS} characters.`
  );

/** The form's fields, by core's rules (`Schedule`, `ScheduleService`). */
export const scheduleSchema = z.object({
  name,
  prompt: z.string().trim().min(1, "Enter the prompt to run."),
  modelId: z.string().min(1, "Choose a model for this schedule."),
  timezone: z.string().trim().min(1, "Choose a time zone, for example Europe/Berlin."),
  instructions: z.string().trim(),
});

/** Everything the form holds. */
export type ScheduleValues = {
  name: string;
  prompt: string;
  modelId: string;
  timezone: string;
  instructions: string;
  reasoning: ReasoningLevel | null;
  onAsk: OnAsk;
  rule: Rule;
  /** The target: a project, or `null` for a standalone schedule. Fixed once created. */
  projectId: string | null;
};

/** A new schedule's form: the device's time zone, a daily rule, and "Deny and continue". */
export function blankValues(timezone = deviceZone()): ScheduleValues {
  return {
    name: "",
    prompt: "",
    modelId: "",
    timezone,
    instructions: "",
    reasoning: null,
    onAsk: "deny",
    rule: DEFAULT_RULE,
    projectId: null,
  };
}

/** The form for a saved schedule. */
export function scheduleValues(schedule: ScheduleResponse): ScheduleValues {
  return {
    name: schedule.name,
    prompt: schedule.prompt,
    modelId: schedule.modelId ?? "",
    timezone: schedule.timezone,
    instructions: schedule.instructions,
    reasoning: schedule.reasoning,
    onAsk: schedule.onAsk,
    rule: readRule(schedule.cron),
    projectId: schedule.projectId,
  };
}

/** The first line of `text`, cut to a schedule name's length. */
function firstLine(text: string): string {
  const line = text.trim().split("\n")[0]?.trim() ?? "";
  return [...line].slice(0, NAME_MAX_CHARS).join("");
}

/**
 * "Schedule this prompt...": the chat's last user message, model, reasoning, and project; a
 * standalone chat also gives its instructions. The name is the chat's title.
 */
export function chatValues(
  chat: Pick<ChatResponse, "title" | "modelId" | "projectId" | "instructions" | "reasoning">,
  messages: Pick<MessageResponse, "role" | "parts">[],
  timezone = deviceZone()
): ScheduleValues {
  const last = [...messages].reverse().find((message) => message.role === "user");
  const prompt = last ? messageText(last.parts).trim() : "";
  return {
    ...blankValues(timezone),
    name: firstLine(chat.title) || firstLine(prompt),
    prompt,
    modelId: chat.modelId ?? "",
    reasoning: chat.reasoning,
    projectId: chat.projectId,
    instructions: chat.projectId === null ? chat.instructions : "",
  };
}

/** A new schedule in a project: the project's default model. */
export function projectValues(
  project: Pick<ProjectResponse, "id" | "defaultModelId">,
  timezone = deviceZone()
): ScheduleValues {
  return { ...blankValues(timezone), projectId: project.id, modelId: project.defaultModelId ?? "" };
}

/**
 * The new schedule form's start: prefilled from a chat, else in a project, else blank. Without a
 * usable model of its own it takes the project's default model, else the first usable one.
 */
export function startValues(from: {
  chat?: Pick<ChatResponse, "title" | "modelId" | "projectId" | "instructions" | "reasoning">;
  messages?: Pick<MessageResponse, "role" | "parts">[];
  project?: Pick<ProjectResponse, "id" | "defaultModelId">;
  models: ModelResponse[];
  timezone?: string;
}): ScheduleValues {
  const timezone = from.timezone ?? deviceZone();
  let values = blankValues(timezone);
  if (from.chat) values = chatValues(from.chat, from.messages ?? [], timezone);
  else if (from.project) values = projectValues(from.project, timezone);
  const model = startModel(from.models, values.modelId, from.project?.defaultModelId);
  return { ...values, modelId: model?.id ?? values.modelId };
}

export type Checked =
  | { ok: true; cron: string; values: ScheduleValues }
  | { ok: false; errors: Record<string, string> };

/** Checks the form by core's rules; the trimmed values and the cron rule when it passes. */
export function checkValues(values: ScheduleValues): Checked {
  const errors: Record<string, string> = {};
  const parsed = scheduleSchema.safeParse(values);
  if (!parsed.success) {
    for (const issue of parsed.error.issues) {
      const field = String(issue.path[0] ?? "");
      errors[field] ??= issue.message;
    }
  }
  Object.assign(errors, ruleErrors(values.rule));
  if (!parsed.success || Object.keys(errors).length > 0) return { ok: false, errors };
  return { ok: true, cron: toCron(values.rule), values: { ...values, ...parsed.data } };
}

/** The `POST /schedules` body. Instructions go only with a standalone schedule. */
export function createBody(values: ScheduleValues, cron: string): CreateScheduleRequest {
  return {
    name: values.name,
    cron,
    timezone: values.timezone,
    prompt: values.prompt,
    modelId: values.modelId,
    onAsk: values.onAsk,
    ...(values.projectId ? { projectId: values.projectId } : { instructions: values.instructions }),
    ...(values.reasoning ? { reasoning: values.reasoning } : {}),
  };
}

/** The `PATCH /schedules/{id}` body: only what changed (`reasoning: null` clears the level). */
export function updateBody(
  schedule: ScheduleResponse,
  values: ScheduleValues,
  cron: string
): UpdateScheduleRequest {
  const body: UpdateScheduleRequest = {};
  if (values.name !== schedule.name) body.name = values.name;
  if (cron !== schedule.cron) body.cron = cron;
  if (values.timezone !== schedule.timezone) body.timezone = values.timezone;
  if (values.prompt !== schedule.prompt) body.prompt = values.prompt;
  if (schedule.projectId === null && values.instructions !== schedule.instructions) {
    body.instructions = values.instructions;
  }
  if (values.modelId && values.modelId !== schedule.modelId) body.modelId = values.modelId;
  if (values.reasoning !== schedule.reasoning) body.reasoning = values.reasoning;
  if (values.onAsk !== schedule.onAsk) body.onAsk = values.onAsk;
  return body;
}

/** A run time in the device's zone ("Mon, Dec 1, 9:00 AM"); `timeZone` and `locale` for tests. */
export function formatRun(
  iso: string,
  options: { timeZone?: string; locale?: string } = {}
): string {
  return new Intl.DateTimeFormat(options.locale, {
    weekday: "short",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
    timeZone: options.timeZone,
  }).format(new Date(iso));
}

/** How the Scheduled list shows where a schedule stands; `detail` is the reason or the error. */
export function scheduleStatus(
  schedule: Pick<
    ScheduleResponse,
    "enabled" | "disabledReason" | "lastStatus" | "lastError" | "lastRunAt"
  >
): { label: string; tone: BadgeTone; detail: string | null } {
  if (schedule.disabledReason) {
    return { label: "Paused", tone: "danger", detail: schedule.disabledReason };
  }
  if (!schedule.enabled) return { label: "Off", tone: "neutral", detail: null };
  switch (schedule.lastStatus) {
    case "ok":
      return { label: "Last run ok", tone: "ok", detail: null };
    case "error":
      return { label: "Last run failed", tone: "danger", detail: schedule.lastError };
    case "awaiting_approval":
      return { label: "Awaiting approval", tone: "warn", detail: null };
    default:
      return { label: "Not run yet", tone: "neutral", detail: null };
  }
}

/** Where a schedule runs: its project's name, or "Standalone". */
export function targetLabel(
  projectId: string | null,
  projects: Pick<ProjectResponse, "id" | "name" | "icon">[] | undefined
): string {
  if (projectId === null) return "Standalone";
  const project = projects?.find((item) => item.id === projectId);
  if (!project) return "Project";
  return project.icon ? `${project.icon} ${project.name}` : project.name;
}

/** Unread runs per schedule, from the latest schedule runs. */
export function unreadRuns(runs: Pick<ChatSummaryResponse, "scheduleId" | "unread">[]) {
  const counts = new Map<string, number>();
  for (const run of runs) {
    if (run.scheduleId && run.unread)
      counts.set(run.scheduleId, (counts.get(run.scheduleId) ?? 0) + 1);
  }
  return counts;
}

/** A tool a run would offer, named for people ("Weather: forecast"). */
export type TargetTool = { server: string; tool: string; mode: Exclude<ToolMode, "off"> };

/**
 * The tools a run in `project` would offer, with the mode each runs with: the project's enabled
 * servers and their listed tools (a server whose list changed asks for every tool until it is
 * reviewed). Off tools are never offered. A standalone schedule runs without tools; unattended
 * runs never get the built-in schedule tools.
 */
export function targetTools(
  project: Pick<ProjectResponse, "toolServerIds"> | null | undefined,
  servers: ToolServerResponse[] | undefined
): TargetTool[] {
  if (!project) return [];
  const chosen = new Set(project.toolServerIds);
  return (servers ?? [])
    .filter((server) => chosen.has(server.id) && server.enabled)
    .flatMap(serverTools);
}

/** A server's listed tools that are not off, with the mode each runs with. */
function serverTools(server: ToolServerResponse): TargetTool[] {
  return (server.tools ?? []).flatMap((tool) => {
    const mode = server.reviewRequired ? "ask" : modeFor(server, tool.name);
    return mode === "off" ? [] : [{ server: server.name, tool: tool.title ?? tool.name, mode }];
  });
}

/** "Weather: forecast, Files: search". */
export function toolNames(tools: TargetTool[]): string {
  return tools.map((item) => `${item.server}: ${item.tool}`).join(", ");
}

/** What the assistant's `schedule_prompt` call asks to set up. */
export type Proposal = { name: string; cron: string; timezone: string; prompt: string };

/**
 * The input of a built-in `schedule_prompt` call, or `null` for any other call and for input
 * without the four text fields (the server refuses that call anyway).
 */
export function readProposal(
  call: Pick<ChatToolCall, "serverId" | "toolName" | "arguments">
): Proposal | null {
  if (call.serverId !== null || call.toolName !== "schedule_prompt") return null;
  let input: unknown = call.arguments;
  if (typeof input === "string") {
    try {
      input = JSON.parse(input);
    } catch {
      return null;
    }
  }
  if (typeof input !== "object" || input === null) return null;
  const fields = input as Record<string, unknown>;
  const { name, cron, timezone, prompt } = fields;
  if (
    typeof name !== "string" ||
    typeof cron !== "string" ||
    typeof timezone !== "string" ||
    typeof prompt !== "string"
  ) {
    return null;
  }
  return { name, cron, timezone, prompt };
}

/**
 * What a schedule made in this chat would run with: the chat's model, its project (or standalone,
 * with the chat's instructions and no tools), and the tools that would run without asking.
 */
export type ScheduleContext = { model: string | null; target: string; autoTools: string };

/** The context of the `schedule_prompt` card for a chat. */
export function scheduleContext(
  model: Pick<ModelResponse, "displayName"> | undefined,
  project: Pick<ProjectResponse, "toolServerIds" | "name" | "icon"> | null | undefined,
  servers: ToolServerResponse[] | undefined
): ScheduleContext {
  return {
    model: model?.displayName ?? null,
    target: project
      ? project.icon
        ? `${project.icon} ${project.name}`
        : project.name
      : "Standalone",
    autoTools: toolNames(targetTools(project, servers).filter((tool) => tool.mode === "auto")),
  };
}
