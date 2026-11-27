// rustauth's account routes, mounted under `/api`. Bodies and statuses follow its spec; the schemas
// come from `@rustauth/contracts` through `@copilot/contracts`.

import {
  type ChangePasswordRequest,
  type LoginRequest,
  type RegisterRequest,
  type SessionResponse,
  type SetupStatusResponse,
  type SignInResponse,
  sessionListBodySchema,
  setupStatusBodySchema,
  signInBodySchema,
  type UpdateProfileRequest,
  type UserResponse,
  userBodySchema,
} from "@copilot/contracts";
import { apiFetch, apiRequest } from "@web/api/client";
import API from "@web/api/endpoints";

/** `GET /auth/setup`: whether the first account is missing, and whether sign-ups are open. */
export async function fetchSetup(signal?: AbortSignal): Promise<SetupStatusResponse> {
  return (await apiFetch(API.auth.setup, setupStatusBodySchema, { signal })).data;
}

/** `GET /auth/me`: the signed-in user. */
export async function fetchMe(signal?: AbortSignal): Promise<UserResponse> {
  return (await apiFetch(API.auth.me, userBodySchema, { signal })).data;
}

/** `POST /auth/login`. */
export async function login(body: LoginRequest): Promise<SignInResponse> {
  return (await apiFetch(API.auth.login, signInBodySchema, { method: "POST", body })).data;
}

/** `POST /auth/register`: creates the account and signs it in. */
export async function register(body: RegisterRequest): Promise<SignInResponse> {
  return (await apiFetch(API.auth.register, signInBodySchema, { method: "POST", body })).data;
}

/** `POST /auth/logout`: revokes the current session on the server. */
export async function logout(): Promise<void> {
  await apiRequest(API.auth.logout, { method: "POST" });
}

/** `PATCH /auth/me`: the caller's display name. */
export async function updateProfile(body: UpdateProfileRequest): Promise<UserResponse> {
  return (await apiFetch(API.auth.me, userBodySchema, { method: "PATCH", body })).data;
}

/**
 * `POST /auth/password`: checks the current password and sets the new one. The server signs out
 * every other session of the account; this one stays signed in.
 */
export async function changePassword(body: ChangePasswordRequest): Promise<void> {
  await apiRequest(API.auth.password, { method: "POST", body });
}

/** `GET /auth/sessions`: the caller's signed-in devices; `current` marks this one. */
export async function fetchSessions(signal?: AbortSignal): Promise<SessionResponse[]> {
  return (await apiFetch(API.auth.sessions, sessionListBodySchema, { signal })).items;
}

/** `DELETE /auth/sessions/{id}`: signs one device out. */
export async function revokeSession(sessionId: string): Promise<void> {
  await apiRequest(API.auth.session(sessionId), { method: "DELETE" });
}

/** `POST /auth/sessions/revoke-others`: signs every other device out. */
export async function revokeOtherSessions(): Promise<void> {
  await apiRequest(API.auth.revokeOtherSessions, { method: "POST" });
}
