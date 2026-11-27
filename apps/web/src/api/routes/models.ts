// The model picker (`GET /models`): enabled, available models on enabled connections. Settings
// change one model with `PATCH /models/{id}`.

import {
  type ModelResponse,
  modelBodySchema,
  modelListBodySchema,
  type UpdateModelRequest,
} from "@copilot/contracts";
import { apiFetch } from "@web/api/client";
import API from "@web/api/endpoints";

/** `GET /models`, grouped by connection name and sorted by display name. */
export async function fetchModels(signal?: AbortSignal): Promise<ModelResponse[]> {
  return (await apiFetch(API.models, modelListBodySchema, { signal })).items;
}

/**
 * `PATCH /models/{id}`: changes only the fields `body` carries; `null` clears the context window
 * or the output limit (the reported value applies again).
 */
export async function updateModel(
  modelId: string,
  body: UpdateModelRequest
): Promise<ModelResponse> {
  return (await apiFetch(API.model(modelId), modelBodySchema, { method: "PATCH", body })).data;
}
