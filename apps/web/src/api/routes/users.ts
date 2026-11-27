// rustauth's user administration (`/admin/users*`), for administrators only. Every change to an
// account signs that account out on every device.

import {
  type CreateUserRequest,
  type UpdateUserRequest,
  type UserResponse,
  userBodySchema,
  userListBodySchema,
} from "@copilot/contracts";
import { apiFetch, apiRequest } from "@web/api/client";
import API from "@web/api/endpoints";

/** Accounts per page of `GET /admin/users` (the server's maximum). */
export const USERS_PAGE = 100;

export type UserPage = { items: UserResponse[]; total: number };

/** `GET /admin/users`: one page of accounts and the total count. */
export async function fetchUsers(offset = 0, signal?: AbortSignal): Promise<UserPage> {
  return apiFetch(API.auth.users, userListBodySchema, {
    query: { offset, limit: USERS_PAGE },
    signal,
  });
}

/** `GET /admin/users/{id}`. */
export async function fetchUser(userId: string, signal?: AbortSignal): Promise<UserResponse> {
  return (await apiFetch(API.auth.user(userId), userBodySchema, { signal })).data;
}

/** `POST /admin/users`: an account with a temporary password the administrator hands over. */
export async function createUser(body: CreateUserRequest): Promise<UserResponse> {
  return (await apiFetch(API.auth.users, userBodySchema, { method: "POST", body })).data;
}

/**
 * `PATCH /admin/users/{id}`: display name, role, disabled, or a new password (which also unlocks
 * the account). The server refuses to leave no enabled administrator (`LAST_ADMINISTRATOR`).
 */
export async function updateUser(userId: string, body: UpdateUserRequest): Promise<UserResponse> {
  return (await apiFetch(API.auth.user(userId), userBodySchema, { method: "PATCH", body })).data;
}

/** `POST /admin/users/{id}/unlock`: ends a lockout after failed sign-ins. */
export async function unlockUser(userId: string): Promise<void> {
  await apiRequest(API.auth.unlockUser(userId), { method: "POST" });
}

/** `DELETE /admin/users/{id}`: deletes the account and all of its Copilot data. */
export async function deleteUser(userId: string): Promise<void> {
  await apiRequest(API.auth.user(userId), { method: "DELETE" });
}
