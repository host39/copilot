// What the model picker shows: enabled, available models grouped by connection, their capability
// flags, the reasoning levels each offers, and which model a new chat starts with.

import type { Capability, ModelResponse, ReasoningLevel } from "@copilot/contracts";
import { shortTokens } from "@web/routes/settings/provider/models";

export type ModelGroup = { providerId: string; provider: string; models: ModelResponse[] };

/** Whether a chat can use `model` now: turned on, and still listed by its provider. */
export function usable(model: ModelResponse): boolean {
  return model.enabled && model.available;
}

/** Usable models grouped by connection, in the order the server sent them (by name). */
export function groupModels(models: ModelResponse[]): ModelGroup[] {
  const groups = new Map<string, ModelGroup>();
  for (const model of models.filter(usable)) {
    const group = groups.get(model.providerId) ?? {
      providerId: model.providerId,
      provider: model.providerName,
      models: [],
    };
    group.models.push(model);
    groups.set(model.providerId, group);
  }
  return [...groups.values()];
}

/** The capability flags the picker shows, in display order, with short labels. */
export const CAPABILITY_LABELS: readonly { name: Capability; label: string }[] = [
  { name: "vision", label: "Images" },
  { name: "pdf", label: "PDFs" },
  { name: "tools", label: "Tools" },
  { name: "audioIn", label: "Audio input" },
  { name: "audioOut", label: "Audio replies" },
  { name: "reasoning", label: "Reasoning" },
];

/** The flags `model` has, in display order. */
export function capabilityNames(model: ModelResponse): Capability[] {
  return CAPABILITY_LABELS.filter(({ name }) => model.capabilities[name]).map(({ name }) => name);
}

/** One line under a model's name: its flags and context window ("Images, Tools · 128K context"). */
export function modelSummary(model: ModelResponse): string {
  const labels = CAPABILITY_LABELS.filter(({ name }) => model.capabilities[name]).map(
    ({ label }) => label
  );
  const flags = labels.length > 0 ? labels.join(", ") : "Text only";
  return model.contextWindow === null
    ? flags
    : `${flags} · ${shortTokens(model.contextWindow)} context`;
}

export const LEVELS: readonly ReasoningLevel[] = ["off", "low", "medium", "high"];

const LEVEL_NAMES: Record<ReasoningLevel, string> = {
  off: "Off",
  low: "Low",
  medium: "Medium",
  high: "High",
};

/**
 * The reasoning levels the picker offers for `model`: none without the `reasoning` flag, the
 * levels the provider reported, or all four when it reported none.
 */
export function reasoningLevels(model: ModelResponse): ReasoningLevel[] {
  if (!model.capabilities.reasoning) return [];
  const listed = model.capabilities.reasoningLevels;
  return LEVELS.filter((level) => listed.length === 0 || listed.includes(level));
}

/** A level's name; `null` is the default ("Model default", or "Model default (medium)"). */
export function levelLabel(level: ReasoningLevel | null, model?: ModelResponse): string {
  if (level !== null) return LEVEL_NAMES[level];
  const own = model?.capabilities.defaultReasoning;
  return own ? `Model default (${LEVEL_NAMES[own].toLowerCase()})` : "Model default";
}

/** `level` when `model` offers it, otherwise the default (`null`). */
export function fitLevel(model: ModelResponse | undefined, level: ReasoningLevel | null) {
  return model && level !== null && reasoningLevels(model).includes(level) ? level : null;
}

/**
 * The first of `candidates` that names a usable model, else the first usable model. The new chat
 * screen passes the project's default model, then the model of the most recently updated chat.
 */
export function startModel(
  models: ModelResponse[],
  ...candidates: (string | null | undefined)[]
): ModelResponse | undefined {
  const list = models.filter(usable);
  for (const id of candidates) {
    const found = id ? list.find((model) => model.id === id) : undefined;
    if (found) return found;
  }
  return list[0];
}
