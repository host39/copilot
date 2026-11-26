// The session token from rustauth's register and login routes. It is a bearer credential: whoever
// holds it acts as the user until it expires or is revoked. It lives in `localStorage`, so the
// user stays signed in across restarts (the server sends a strict CSP and the SPA loads no
// third-party scripts), and in memory when storage is blocked. Signing out revokes it on the server
// (`POST /auth/logout`) before it is cleared here.

import { readItem, removeItem, writeItem } from "@web/storage";

const TOKEN_KEY = "copilot.token";

type Listener = () => void;

const listeners = new Set<Listener>();
let loaded = false;
let current: string | null = null;

function notify(): void {
  for (const listener of listeners) listener();
}

/** The stored token, or `null` when signed out. */
export function getToken(): string | null {
  if (!loaded) {
    current = readItem(TOKEN_KEY);
    loaded = true;
  }
  return current;
}

export function setToken(token: string): void {
  loaded = true;
  current = token;
  writeItem(TOKEN_KEY, token);
  notify();
}

export function clearToken(): void {
  const had = getToken() !== null;
  current = null;
  removeItem(TOKEN_KEY);
  if (had) notify();
}

/** Calls `listener` whenever the token changes, here or in another tab. Returns the unsubscribe. */
export function subscribeToken(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * Follows a `storage` event: another tab signed in or out, so no tab keeps acting on a revoked
 * token or misses a new one.
 */
export function followStorage(event: Pick<StorageEvent, "key">): void {
  if (event.key !== TOKEN_KEY && event.key !== null) return;
  const next = readItem(TOKEN_KEY);
  if (loaded && next === current) return;
  loaded = true;
  current = next;
  notify();
}

globalThis.addEventListener?.("storage", followStorage);
