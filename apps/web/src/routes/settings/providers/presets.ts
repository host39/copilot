// The provider presets a user picks from (the spec's Providers and Models table), and the form
// that turns a pick plus what the user typed into `POST /providers`.

import type {
  CreateProviderRequest,
  Preset,
  ProviderKind,
  ProviderResponse,
  RefreshResponse,
} from "@copilot/contracts";
import { describeError } from "@web/api/errors";
import { createProvider, refreshProvider } from "@web/api/routes/providers";
import { plural } from "@web/routes/settings/format";
import { z } from "zod";

export type Choice = {
  id: string;
  label: string;
  kind: ProviderKind;
  preset: Preset;
  /** What the form asks for: a key (hosted providers), an address (Ollama), or both (custom). */
  asks: "key" | "url" | "both";
  /** The address field's example. */
  urlHint?: string;
};

/** A key for hosted providers, an address for Ollama, both for compatible servers. */
export const CHOICES: readonly Choice[] = [
  { id: "openai", label: "OpenAI", kind: "openai", preset: "openai", asks: "key" },
  { id: "anthropic", label: "Anthropic", kind: "anthropic", preset: "anthropic", asks: "key" },
  { id: "gemini", label: "Gemini", kind: "openai", preset: "gemini", asks: "key" },
  { id: "openrouter", label: "OpenRouter", kind: "openai", preset: "openrouter", asks: "key" },
  {
    id: "ollama",
    label: "Ollama",
    kind: "openai",
    preset: "ollama",
    asks: "url",
    urlHint: "http://localhost:11434/v1",
  },
  {
    id: "custom-openai",
    label: "OpenAI-compatible",
    kind: "openai",
    preset: "custom",
    asks: "both",
    urlHint: "https://llm.example.com/v1",
  },
  {
    id: "custom-anthropic",
    label: "Anthropic-compatible",
    kind: "anthropic",
    preset: "custom",
    asks: "both",
    urlHint: "https://gateway.example.com",
  },
];

/** The choice of an existing connection's kind and preset. */
export function choice(kind: ProviderKind, preset: Preset): Choice {
  return (
    CHOICES.find((item) => item.kind === kind && item.preset === preset) ??
    (CHOICES.find((item) => item.kind === kind && item.preset === "custom") as Choice)
  );
}

/** Longest connection name the server accepts (`NAME_MAX_CHARS`). */
export const NAME_MAX_CHARS = 100;

const chars = (value: string) => Array.from(value).length;

/** An absolute `http` or `https` URL. */
function isHttp(value: string): boolean {
  try {
    const url = new URL(value);
    return url.protocol === "http:" || url.protocol === "https:";
  } catch {
    return false;
  }
}

/** Optional text: blank reads as left out. */
const optional = z
  .string()
  .trim()
  .transform((value) => value || undefined);

/**
 * The add-provider form for `pick`: a name (the preset's label when blank), the address (required
 * for compatible servers, optional for Ollama, not asked for hosted providers), and the key
 * (required for hosted providers unless `keyStored`: the first-run setup edits a saved connection
 * whose key a blank field keeps). It parses to the `POST /providers` body.
 */
export function providerSchema(pick: Choice, keyStored = false) {
  return z
    .object({
      name: z
        .string()
        .trim()
        .refine((value) => chars(value) <= NAME_MAX_CHARS, "Use at most 100 characters."),
      baseUrl: optional.refine(
        (value) => value === undefined || isHttp(value),
        "Enter a full http:// or https:// address."
      ),
      apiKey: optional,
    })
    .superRefine((input, context) => {
      if (pick.asks === "both" && input.baseUrl === undefined) {
        context.addIssue({
          code: "custom",
          path: ["baseUrl"],
          message: "Enter the server address.",
        });
      }
      if (pick.asks === "key" && input.apiKey === undefined && !keyStored) {
        context.addIssue({
          code: "custom",
          path: ["apiKey"],
          message: `Paste your ${pick.label} API key.`,
        });
      }
    })
    .transform(
      (input): CreateProviderRequest => ({
        kind: pick.kind,
        preset: pick.preset,
        name: input.name || pick.label,
        ...(pick.asks !== "key" && input.baseUrl ? { baseUrl: input.baseUrl } : {}),
        ...(pick.asks !== "url" && input.apiKey ? { apiKey: input.apiKey } : {}),
      })
    );
}

/** A new connection and its first test: what the refresh found, or why it failed. */
export type Connected = {
  provider: ProviderResponse;
  refresh?: RefreshResponse;
  /** Why listing the models failed (a rejected key, an unreachable server). */
  problem?: string;
};

/** What the add-provider screen hands the connection's page (`location.state`). */
export type ConnectState = Pick<Connected, "refresh" | "problem">;

/**
 * Saves a connection, then tests it by listing its models (`POST /providers/{id}/refresh`). A
 * failed test keeps the connection, so the user can fix the key or address instead of starting
 * over.
 */
export async function connect(body: CreateProviderRequest): Promise<Connected> {
  const provider = await createProvider(body);
  try {
    return { provider, refresh: await refreshProvider(provider.id) };
  } catch (error) {
    return { provider, problem: describeError(error).message };
  }
}

/** What a refresh found, in words ("Found 12 models. All are enabled."). */
export function refreshSummary(report: RefreshResponse): string {
  const listed = report.added + report.updated;
  const parts = [`Found ${plural(listed, "model")}`];
  if (report.added > 0 && report.updated > 0) parts.push(`${report.added} new`);
  if (report.unavailable > 0) parts.push(`${report.unavailable} no longer listed`);
  const found = `${parts.join(", ")}.`;
  if (report.enabled > 0 && report.enabled === listed) {
    return `${found} ${listed === 1 ? "It is" : "All are"} enabled.`;
  }
  if (report.enabled > 0) return `${found} ${plural(report.enabled, "new model")} enabled.`;
  if (report.added > 0) return `${found} Choose the ones to enable.`;
  return found;
}
