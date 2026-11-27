// How long to wait before connecting again, shared by the chat stream's reattach and the events
// reader.

/** The wait before reconnect `attempt` (from 0): 1, 2, 4, 8, then every 15 seconds. */
export function backoff(attempt: number): number {
  return Math.min(1_000 * 2 ** Math.max(0, attempt), 15_000);
}
