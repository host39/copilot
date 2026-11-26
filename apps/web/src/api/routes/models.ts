// The model picker (`GET /models`): enabled, available models on enabled connections.

import { type ModelResponse, modelListBodySchema } from "@copilot/contracts";
import { apiFetch } from "@web/api/client";
import API from "@web/api/endpoints";

/** `GET /models`, grouped by connection name and sorted by display name. */
export async function fetchModels(signal?: AbortSignal): Promise<ModelResponse[]> {
  return (await apiFetch(API.models, modelListBodySchema, { signal })).items;
}
