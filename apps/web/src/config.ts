// Where the Copilot server lives. The server URL is the origin (plus an optional path prefix) that
// serves the SPA, `/health`, and the API under `/api`.
//
// - Web builds talk to the server that served the page: the server URL is "" and requests go to
//   relative `/api/...` URLs (the dev server proxies `/api` and `/health` to the API). Setting
//   `PUBLIC_API_URL` at build time names another server instead, for example a worktree's API.
// - Native builds (Tauri) have no server of their own: the Connect screen stores the URL the user
//   entered, and every request is built from it at run time.

import { isTauri } from "@tauri-apps/api/core";
import { clearToken } from "@web/auth/token";
import { readItem, removeItem, writeItem } from "@web/storage";

/** API routes live under this prefix of the server URL. */
export const API_PREFIX = "/api";

const SERVER_URL_KEY = "copilot.server-url";

/** Whether this bundle runs inside the Tauri shell (Linux, Android). */
export function isNative(): boolean {
  return isTauri();
}

/**
 * Checks a server URL and returns it without a trailing slash, or `undefined` when it is not an
 * absolute `http` or `https` URL without credentials, query, or fragment.
 */
export function parseServerUrl(value: string): string | undefined {
  let url: URL;
  try {
    url = new URL(value.trim());
  } catch {
    return undefined;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return undefined;
  if (url.username || url.password || url.search || url.hash) return undefined;
  return `${url.origin}${url.pathname.replace(/\/+$/, "")}`;
}

/** The server URL fixed at build time: "" (same origin) unless `PUBLIC_API_URL` is set. */
export function buildServerUrl(): string {
  const value = import.meta.env.PUBLIC_API_URL?.trim();
  return value ? (parseServerUrl(value) ?? "") : "";
}

/**
 * The server URL requests are built from: the stored URL in native builds (`undefined` until the
 * user connects), the build-time value otherwise.
 */
export function serverUrl(): string | undefined {
  if (!isNative()) return buildServerUrl();
  const stored = readItem(SERVER_URL_KEY);
  return stored === null ? undefined : parseServerUrl(stored);
}

/**
 * Stores the server URL of a native build. A different server never sees the previous server's
 * token, so changing it signs the user out.
 */
export function saveServerUrl(value: string): string {
  const url = parseServerUrl(value);
  if (url === undefined) throw new Error("Enter a full http:// or https:// address.");
  if (url !== serverUrl()) clearToken();
  writeItem(SERVER_URL_KEY, url);
  return url;
}

/** Forgets the stored server URL (native builds), which also signs the user out. */
export function forgetServerUrl(): void {
  clearToken();
  removeItem(SERVER_URL_KEY);
}
