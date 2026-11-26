// rustauth's account routes, mounted under `/api`. Bodies and statuses follow its spec; the schemas
// come from `@rustauth/contracts` through `@copilot/contracts`.

import {
  type LoginRequest,
  type RegisterRequest,
  type SetupStatusResponse,
  type SignInResponse,
  setupStatusBodySchema,
  signInBodySchema,
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
