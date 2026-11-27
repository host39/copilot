// Projects (`/projects`): reusable context for many chats. Their files are in `files.ts`.

import {
  type CreateProjectRequest,
  type ProjectDetailsResponse,
  type ProjectListQuery,
  type ProjectResponse,
  projectBodySchema,
  projectDetailsBodySchema,
  projectListBodySchema,
  type UpdateProjectRequest,
} from "@copilot/contracts";
import { apiFetch, apiRequest } from "@web/api/client";
import API from "@web/api/endpoints";

/** `GET /projects`: most recently updated first, narrowed by `query` (`pinned`, `limit`). */
export async function fetchProjects(
  query: ProjectListQuery = {},
  signal?: AbortSignal
): Promise<ProjectResponse[]> {
  return (await apiFetch(API.projects, projectListBodySchema, { query, signal })).items;
}

/** `GET /projects/{id}`: the project with its capacity bar. */
export async function fetchProject(
  projectId: string,
  signal?: AbortSignal
): Promise<ProjectDetailsResponse> {
  return (await apiFetch(API.project(projectId), projectDetailsBodySchema, { signal })).data;
}

/** `POST /projects`. */
export async function createProject(body: CreateProjectRequest): Promise<ProjectResponse> {
  return (await apiFetch(API.projects, projectBodySchema, { method: "POST", body })).data;
}

/** `PATCH /projects/{id}`: changes only the fields `body` carries; `null` clears one. */
export async function updateProject(
  projectId: string,
  body: UpdateProjectRequest
): Promise<ProjectResponse> {
  return (await apiFetch(API.project(projectId), projectBodySchema, { method: "PATCH", body }))
    .data;
}

/** `DELETE /projects/{id}`: the project with its chats, files, and schedules. */
export async function deleteProject(projectId: string): Promise<void> {
  await apiRequest(API.project(projectId), { method: "DELETE" });
}
