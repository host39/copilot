// A connection's models in Settings: search and filter, the capability labels, and the model
// editor's form, which sends only what changed.

import type {
  Capability,
  CapabilityOverrides,
  ModelResponse,
  ProviderUsage,
  UpdateModelRequest,
} from "@copilot/contracts";
import { plural } from "@web/routes/settings/format";
import { z } from "zod";

/** The capability flags in display order, with core's labels. */
export const CAPABILITIES: readonly { name: Capability; label: string }[] = [
  { name: "vision", label: "Image input" },
  { name: "pdf", label: "PDF input" },
  { name: "tools", label: "Tool use" },
  { name: "audioIn", label: "Audio input" },
  { name: "audioOut", label: "Audio output" },
  { name: "reasoning", label: "Reasoning" },
];

/** Which models the list shows. */
export type Show = "all" | "enabled" | "disabled";

/**
 * The models whose display name or id contains every word of `query` (any case), narrowed by
 * `show`: models the provider still lists first, then by display name.
 */
export function filterModels(
  models: readonly ModelResponse[],
  query: string,
  show: Show
): ModelResponse[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return models
    .filter((model) => show === "all" || model.enabled === (show === "enabled"))
    .filter((model) => {
      const haystack = `${model.displayName} ${model.remoteId}`.toLowerCase();
      return words.every((word) => haystack.includes(word));
    })
    .sort(
      (a, b) =>
        Number(b.available) - Number(a.available) ||
        a.displayName.localeCompare(b.displayName, "en", { sensitivity: "base" })
    );
}

/** A capability override as the editor shows it: the detected value, or forced on or off. */
export type OverrideChoice = "default" | "on" | "off";

export function overrideChoice(model: ModelResponse, capability: Capability): OverrideChoice {
  const value = model.overrides[capability];
  if (value === null) return "default";
  return value ? "on" : "off";
}

/** A token count in short form ("128K", "1M", "1.5M"). */
export function shortTokens(count: number): string {
  if (count >= 1_000_000) return `${Number((count / 1_000_000).toFixed(1))}M`;
  if (count >= 1_000) return `${Math.round(count / 1_000)}K`;
  return String(count);
}

/** The largest token limit the server stores (a Postgres `integer`). */
const TOKEN_LIMIT_MAX = 2_147_483_647;

/** An optional token limit: blank clears it; otherwise a whole number the server accepts. */
function tokens(label: string) {
  return z
    .string()
    .trim()
    .refine((value) => /^\d*$/.test(value.replaceAll(",", "")), {
      message: "Enter a whole number of tokens.",
      abort: true,
    })
    .transform((value) => {
      const digits = value.replaceAll(",", "");
      return digits === "" ? null : Number(digits);
    })
    .refine((value) => value !== 0, { message: `${label} must be greater than zero.`, abort: true })
    .refine((value) => value === null || value <= TOKEN_LIMIT_MAX, {
      message: `${label} must be at most ${TOKEN_LIMIT_MAX}.`,
      abort: true,
    });
}

const choice = z.enum(["default", "on", "off"]).optional();

/**
 * The overrides the editor's choices make: a choice the form left out keeps the stored value, and
 * a locked flag is never overridden (the server refuses it).
 */
function pickOverrides(
  model: ModelResponse,
  picked: Partial<Record<Capability, OverrideChoice>>
): CapabilityOverrides {
  const locked = new Set(model.locks.map((lock) => lock.capability));
  const overrides: CapabilityOverrides = { ...model.overrides };
  for (const { name } of CAPABILITIES) {
    const value = picked[name];
    if (locked.has(name)) overrides[name] = null;
    else if (value !== undefined) overrides[name] = value === "default" ? null : value === "on";
  }
  return overrides;
}

/**
 * The model editor for `model`: display name, context window, max output tokens, and an override
 * per capability that is not locked. It parses to a `PATCH /models/{id}` body with only the fields
 * that changed (`{}` when nothing did).
 */
export function modelSchema(model: ModelResponse) {
  return z
    .object({
      displayName: z
        .string()
        .trim()
        .min(1, "Enter a name.")
        .refine((value) => Array.from(value).length <= 100, "Use at most 100 characters."),
      contextWindow: tokens("The context window"),
      maxOutputTokens: tokens("Max output tokens"),
      vision: choice,
      pdf: choice,
      tools: choice,
      audioIn: choice,
      audioOut: choice,
      reasoning: choice,
    })
    .refine(
      (input) =>
        input.contextWindow === null ||
        input.maxOutputTokens === null ||
        input.maxOutputTokens < input.contextWindow,
      {
        message: "Max output tokens must be less than the context window.",
        path: ["maxOutputTokens"],
      }
    )
    .transform((input): UpdateModelRequest => {
      const body: UpdateModelRequest = {};
      if (input.displayName !== model.displayName) body.displayName = input.displayName;
      if (input.contextWindow !== model.contextWindow) body.contextWindow = input.contextWindow;
      if (input.maxOutputTokens !== model.maxOutputTokens) {
        body.maxOutputTokens = input.maxOutputTokens;
      }
      const overrides = pickOverrides(model, input);
      if (CAPABILITIES.some(({ name }) => overrides[name] !== model.overrides[name])) {
        body.capabilityOverrides = overrides;
      }
      return body;
    });
}

/** The delete confirmation's sentence about what uses the connection's models. */
export function usageText(usage: ProviderUsage): string {
  const parts = [
    usage.chats > 0 ? plural(usage.chats, "chat") : "",
    usage.projects > 0 ? plural(usage.projects, "project") : "",
    usage.schedules > 0 ? plural(usage.schedules, "schedule") : "",
  ].filter(Boolean);
  if (parts.length === 0) return "No chat, project, or schedule uses its models.";
  const list = new Intl.ListFormat("en", { type: "conjunction" }).format(parts);
  const kept =
    usage.chats > 0 || usage.projects > 0
      ? " Chats and projects keep their messages and settings, without a model until you pick another."
      : "";
  const paused = usage.schedules > 0 ? " Schedules that run one of them are turned off." : "";
  return `Its models are used by ${list}.${kept}${paused}`;
}
