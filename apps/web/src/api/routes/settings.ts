// The caller's personal instructions (`/settings`), sent with every chat.

import {
  type SettingsResponse,
  settingsBodySchema,
  type UpdateSettingsRequest,
} from "@copilot/contracts";
import { apiFetch } from "@web/api/client";
import API from "@web/api/endpoints";

/** `GET /settings`. */
export async function fetchSettings(signal?: AbortSignal): Promise<SettingsResponse> {
  return (await apiFetch(API.settings, settingsBodySchema, { signal })).data;
}

/** `PATCH /settings`: changes only the fields `body` carries. */
export async function updateSettings(body: UpdateSettingsRequest): Promise<SettingsResponse> {
  return (await apiFetch(API.settings, settingsBodySchema, { method: "PATCH", body })).data;
}
