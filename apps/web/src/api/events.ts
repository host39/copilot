// The caller's live updates (`GET /events`): which chats changed, runs that start and finish,
// answers that wait for approval, and schedules that changed. `EventSource` cannot send the bearer
// token, so `watchEvents` reads the stream over the client's `fetch` and reconnects the way
// `EventSource` does: after a dropped stream (1, 2, 4, 8, then every 15 seconds), at once when the
// device is back online or the page visible, slowly while the server has no event stream (an
// older server answers `404` or `501`), and never after the session ended (`401`).
//
// One reader runs per signed-in shell (`useEventStream`); screens listen with `onUserEvent`.

import { type UserEvent, userEventSchema } from "@copilot/contracts";
import { ApiError, apiStream, isAbort } from "@web/api/client";
import API from "@web/api/endpoints";
import { backoff } from "@web/api/retry";
import { keys } from "@web/api/routes/keys";
import { log } from "@web/logging";

/** The wait before asking a server without an event stream again. */
export const MISSING_RETRY_MS = 5 * 60_000;
/** The wait after the server closed a stream on its own (a restart, a proxy's timeout). */
export const CLOSED_RETRY_MS = 1_000;

/**
 * What to do after a stream ended (`error` undefined) or failed: wait that many milliseconds and
 * connect again, or stop (`null`). `failures` counts the failed tries since the last open stream.
 */
export function nextTry(error: unknown, failures: number): number | null {
  if (error === undefined) return CLOSED_RETRY_MS;
  if (error instanceof ApiError) {
    if (error.status === 401) return null;
    if (error.status === 429) {
      return Math.max((error.retryAfterSecs ?? 0) * 1_000, backoff(failures));
    }
    if ((error.status >= 400 && error.status < 500) || error.status === 501) {
      return MISSING_RETRY_MS;
    }
  }
  return backoff(failures);
}

/** Resolves after `ms`, or early once `wake` fires or `signal` aborts. */
export type Wait = (ms: number, wake: AbortSignal) => Promise<void>;

const sleep: Wait = (ms, wake) =>
  new Promise((resolve) => {
    if (wake.aborted) {
      resolve();
      return;
    }
    const timer = setTimeout(resolve, ms);
    wake.addEventListener(
      "abort",
      () => {
        clearTimeout(timer);
        resolve();
      },
      { once: true }
    );
  });

export type WatchOptions = {
  onEvent: (event: UserEvent) => void;
  /** A stream opened; `again` after an earlier one dropped (events may have been missed). */
  onOpen?: (again: boolean) => void;
  /** Stops the reader. */
  signal: AbortSignal;
  /**
   * Cuts a wait short: call the returned function when it is worth trying at once (the device is
   * online again, the page visible). Tests pass their own.
   */
  wakes?: (wake: () => void) => () => void;
  /** Waits between tries (tests pass their own). */
  wait?: Wait;
};

/** Wakes on `online` and on the page becoming visible. */
function browserWakes(wake: () => void): () => void {
  if (typeof window === "undefined" || typeof document === "undefined") return () => undefined;
  const visible = () => {
    if (document.visibilityState === "visible") wake();
  };
  window.addEventListener("online", wake);
  document.addEventListener("visibilitychange", visible);
  return () => {
    window.removeEventListener("online", wake);
    document.removeEventListener("visibilitychange", visible);
  };
}

/** How one connection ended: closed by the server (`error` undefined), failed, or aborted. */
type Ending = { aborted: true } | { aborted: false; error: unknown };

/** Reads one stream to its end. */
async function readOnce(
  onEvent: (event: UserEvent) => void,
  onOpen: () => void,
  signal: AbortSignal
): Promise<Ending> {
  try {
    await apiStream(API.events, userEventSchema, onEvent, { signal, onOpen });
    return { aborted: false, error: undefined };
  } catch (error) {
    if (signal.aborted || isAbort(error)) return { aborted: true };
    return { aborted: false, error };
  }
}

/** Reads the caller's event stream until `signal` aborts or the session ends; see the file comment. */
export async function watchEvents({
  onEvent,
  onOpen,
  signal,
  wakes = browserWakes,
  wait = sleep,
}: WatchOptions): Promise<void> {
  let failures = 0;
  let opened = false;
  let waking = new AbortController();
  const stopWakes = wakes(() => waking.abort());
  const stop = () => waking.abort();
  signal.addEventListener("abort", stop, { once: true });
  const opening = () => {
    onOpen?.(opened);
    opened = true;
    failures = 0;
  };
  try {
    while (!signal.aborted) {
      const ending = await readOnce(onEvent, opening, signal);
      if (ending.aborted) return;
      const delay = nextTry(ending.error, failures);
      if (delay === null) return;
      if (ending.error !== undefined) {
        failures += 1;
        log("warn", "web.events.stream.failed", {
          status: ending.error instanceof ApiError ? ending.error.status : undefined,
          retryMs: delay,
        });
      }
      waking = new AbortController();
      if (signal.aborted) return;
      await wait(delay, waking.signal);
    }
  } finally {
    stopWakes();
    signal.removeEventListener("abort", stop);
  }
}

/** Cached answers to refresh: a key and its children, or that key alone (`exact`). */
export type Stale = { queryKey: readonly unknown[]; exact?: boolean };

const LISTS: Stale[] = [{ queryKey: [...keys.chats, "list"] }, { queryKey: keys.runs }];

/**
 * The cached answers an event makes stale. A chat's messages are its page's business (the page
 * follows its own runs, see `useTurn`): refreshing them here would mark the chat read.
 */
export function staleKeys(event: UserEvent): Stale[] {
  switch (event.type) {
    case "chat.updated":
      return [...LISTS, { queryKey: keys.chat(event.chatId), exact: true }];
    case "run.started":
    case "run.finished":
    case "approval.pending":
      return LISTS;
    case "schedule.updated":
      return [
        { queryKey: [...keys.schedules, "list"] },
        { queryKey: keys.schedule(event.scheduleId) },
      ];
  }
}

/** What to refresh after a dropped stream: anything may have changed meanwhile. */
export const CATCH_UP: readonly Stale[] = [...LISTS, { queryKey: [...keys.schedules, "list"] }];

type Listener = (event: UserEvent) => void;
const listeners = new Set<Listener>();

/** Hears every user event while the shell's reader runs; returns the unsubscribe. */
export function onUserEvent(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Hands an event to every listener (the shell's reader, and tests). */
export function publish(event: UserEvent): void {
  for (const listener of [...listeners]) {
    try {
      listener(event);
    } catch (error) {
      log("error", "web.events.listener.failed", { type: event.type, error: String(error) });
    }
  }
}
