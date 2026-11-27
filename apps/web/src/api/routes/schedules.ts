// Schedules: prompts that run at set times (`/schedules`). A schedule's runs are the chats it
// created, read with `fetchChats({ scheduleId })`.

import {
  type CreateScheduleRequest,
  type ScheduleListQuery,
  type SchedulePreviewRequest,
  type SchedulePreviewResponse,
  type ScheduleResponse,
  scheduleBodySchema,
  scheduleListBodySchema,
  schedulePreviewBodySchema,
  scheduleRunBodySchema,
  type UpdateScheduleRequest,
} from "@copilot/contracts";
import { apiFetch, apiRequest } from "@web/api/client";
import API from "@web/api/endpoints";

/** `GET /schedules`: the caller's schedules, oldest first; `projectId` keeps one project's. */
export async function fetchSchedules(
  query: ScheduleListQuery = {},
  signal?: AbortSignal
): Promise<ScheduleResponse[]> {
  return (await apiFetch(API.schedules, scheduleListBodySchema, { query, signal })).items;
}

/** `GET /schedules/{id}`. */
export async function fetchSchedule(
  scheduleId: string,
  signal?: AbortSignal
): Promise<ScheduleResponse> {
  return (await apiFetch(API.schedule(scheduleId), scheduleBodySchema, { signal })).data;
}

/** `POST /schedules`: the new schedule, turned on. */
export async function createSchedule(body: CreateScheduleRequest): Promise<ScheduleResponse> {
  return (await apiFetch(API.schedules, scheduleBodySchema, { method: "POST", body })).data;
}

/** `PATCH /schedules/{id}`: changes only the fields `body` carries (`enabled` turns it on or off). */
export async function updateSchedule(
  scheduleId: string,
  body: UpdateScheduleRequest
): Promise<ScheduleResponse> {
  return (await apiFetch(API.schedule(scheduleId), scheduleBodySchema, { method: "PATCH", body }))
    .data;
}

/** `DELETE /schedules/{id}`: the chats of its runs stay. */
export async function deleteSchedule(scheduleId: string): Promise<void> {
  await apiRequest(API.schedule(scheduleId), { method: "DELETE" });
}

/**
 * `POST /schedules/{id}/run`: runs it now, unattended, like a scheduled fire (the next run stays).
 * Returns the run's chat; open it to follow the answer.
 */
export async function runSchedule(scheduleId: string): Promise<string> {
  return (await apiFetch(API.scheduleRun(scheduleId), scheduleRunBodySchema, { method: "POST" }))
    .data.chatId;
}

/** `POST /schedules/preview`: the rule in plain words and its next 3 runs. */
export async function previewSchedule(
  body: SchedulePreviewRequest,
  signal?: AbortSignal
): Promise<SchedulePreviewResponse> {
  return (
    await apiFetch(API.schedulePreview, schedulePreviewBodySchema, {
      method: "POST",
      body,
      signal,
    })
  ).data;
}
