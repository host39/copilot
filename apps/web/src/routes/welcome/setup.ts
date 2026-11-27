// The first-run setup's state: (1) pick a preset and enter a key or address, which saves the
// connection; (2) test it by listing its models; (3) choose models. Going back to fix the key
// updates the saved connection instead of adding a second one.

import type {
  CreateProviderRequest,
  ModelResponse,
  ProviderResponse,
  RefreshResponse,
} from "@copilot/contracts";
import {
  createProvider,
  deleteProvider,
  fetchProviders,
  updateProvider,
} from "@web/api/routes/providers";
import path from "@web/routes/path";
import { filterModels } from "@web/routes/settings/provider/models";
import { CHOICES, type Choice } from "@web/routes/settings/providers/presets";

export type Test =
  | { status: "idle" }
  | { status: "running" }
  | { status: "passed"; report: RefreshResponse }
  | { status: "failed"; problem: string };

export type SetupState = {
  step: 0 | 1 | 2;
  pick: Choice;
  /** The connection step 1 saved; coming back updates it. */
  provider?: ProviderResponse;
  test: Test;
};

export type SetupEvent =
  | { type: "pick"; pick: Choice }
  /** Step 1 saved the connection: step 2 starts testing it. */
  | { type: "saved"; provider: ProviderResponse }
  | { type: "retest" }
  | { type: "tested"; report: RefreshResponse }
  | { type: "failed"; problem: string }
  | { type: "back" }
  | { type: "next" };

export const initialSetup: SetupState = {
  step: 0,
  pick: CHOICES[0] as Choice,
  test: { status: "idle" },
};

/** Whether step 1 edits the saved connection of the same preset (a blank key keeps its key). */
export function keepsKey(state: SetupState): boolean {
  return (
    state.provider !== undefined &&
    state.provider.kind === state.pick.kind &&
    state.provider.preset === state.pick.preset &&
    state.provider.key === "set"
  );
}

/** A test's progress counts only while the test step shows. */
function testing(state: SetupState, test: Test): SetupState {
  return state.step === 1 ? { ...state, test } : state;
}

export function setupReducer(state: SetupState, event: SetupEvent): SetupState {
  switch (event.type) {
    case "pick":
      return state.step === 0 ? { ...state, pick: event.pick } : state;
    case "saved":
      return { ...state, step: 1, provider: event.provider, test: { status: "running" } };
    case "retest":
      return testing(state, { status: "running" });
    case "tested":
      return testing(state, { status: "passed", report: event.report });
    case "failed":
      return testing(state, { status: "failed", problem: event.problem });
    case "back":
      // Back from the models keeps the test's outcome; back from the test edits the connection.
      if (state.step === 2) return { ...state, step: 1 };
      return state.step === 1 ? { ...state, step: 0, test: { status: "idle" } } : state;
    case "next":
      return state.step === 1 && state.test.status === "passed" ? { ...state, step: 2 } : state;
  }
}

/**
 * Saves step 1: creates the connection, or updates the one saved before with a new key or
 * address. A different preset replaces it (kind and preset cannot change).
 */
export async function saveConnection(
  saved: ProviderResponse | undefined,
  body: CreateProviderRequest
): Promise<ProviderResponse> {
  if (saved === undefined) return createProvider(body);
  if (saved.kind !== body.kind || saved.preset !== body.preset) {
    await deleteProvider(saved.id);
    return createProvider(body);
  }
  const changes = {
    ...(body.baseUrl !== undefined && body.baseUrl !== saved.baseUrl
      ? { baseUrl: body.baseUrl }
      : {}),
    ...(body.apiKey !== undefined ? { apiKey: body.apiKey } : {}),
  };
  return Object.keys(changes).length === 0 ? saved : updateProvider(saved.id, changes);
}

/** The model a new chat opens with after the setup: the first one turned on, in list order. */
export function firstModel(models: readonly ModelResponse[]): ModelResponse | undefined {
  return filterModels(models, "", "enabled")[0];
}

/** What the new chat screen reads from `location.state` after the setup. */
export type StartState = { modelId?: string };

/**
 * Where a sign-in goes: the first-run setup while the user has no provider connection (every
 * later user sees it on their first sign-in, and again until they connect one), else `target`.
 * When the check fails, the sign-in goes on to `target`.
 */
export async function landing(target: string): Promise<string> {
  try {
    return (await fetchProviders()).length === 0 ? path.welcome : target;
  } catch {
    return target;
  }
}
