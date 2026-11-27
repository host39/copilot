// Provider connections (`/providers`) and their models. Keys only go in: answers say whether one
// is stored (`key`), never what it is.

import {
  type CreateProviderRequest,
  type ModelResponse,
  modelListBodySchema,
  type ProviderDetailsResponse,
  type ProviderResponse,
  providerBodySchema,
  providerDetailsBodySchema,
  providerListBodySchema,
  type RefreshResponse,
  refreshBodySchema,
  type UpdateProviderRequest,
} from "@copilot/contracts";
import { apiFetch, apiRequest } from "@web/api/client";
import API from "@web/api/endpoints";

/** `GET /providers`: the caller's connections, oldest first. */
export async function fetchProviders(signal?: AbortSignal): Promise<ProviderResponse[]> {
  return (await apiFetch(API.providers, providerListBodySchema, { signal })).items;
}

/** `GET /providers/{id}`: the connection with the usage counts its delete confirmation shows. */
export async function fetchProvider(
  providerId: string,
  signal?: AbortSignal
): Promise<ProviderDetailsResponse> {
  return (await apiFetch(API.provider(providerId), providerDetailsBodySchema, { signal })).data;
}

/** `POST /providers`. The connection has no models until its first refresh. */
export async function createProvider(body: CreateProviderRequest): Promise<ProviderResponse> {
  return (await apiFetch(API.providers, providerBodySchema, { method: "POST", body })).data;
}

/**
 * `PATCH /providers/{id}`: changes only the fields `body` carries. A blank `apiKey` removes the
 * stored key; a base URL on another origin drops it unless `apiKey` sets a new one.
 */
export async function updateProvider(
  providerId: string,
  body: UpdateProviderRequest
): Promise<ProviderResponse> {
  return (await apiFetch(API.provider(providerId), providerBodySchema, { method: "PATCH", body }))
    .data;
}

/** `DELETE /providers/{id}`: removes the connection and its models. */
export async function deleteProvider(providerId: string): Promise<void> {
  await apiRequest(API.provider(providerId), { method: "DELETE" });
}

/**
 * `POST /providers/{id}/refresh`: lists the provider's models, which also tests the connection.
 * A provider that lists 20 models or fewer gets all of them enabled.
 */
export async function refreshProvider(providerId: string): Promise<RefreshResponse> {
  return (await apiFetch(API.providerRefresh(providerId), refreshBodySchema, { method: "POST" }))
    .data;
}

/** `GET /providers/{id}/models`: every known model of the connection, enabled or not. */
export async function fetchProviderModels(
  providerId: string,
  signal?: AbortSignal
): Promise<ModelResponse[]> {
  return (await apiFetch(API.providerModels(providerId), modelListBodySchema, { signal })).items;
}
