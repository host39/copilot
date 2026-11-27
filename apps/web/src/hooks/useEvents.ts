import type { UserEvent } from "@copilot/contracts";
import { useQueryClient } from "@tanstack/react-query";
import { CATCH_UP, onUserEvent, publish, staleKeys, watchEvents } from "@web/api/events";
import { useEffect, useRef } from "react";

/**
 * Keeps the signed-in shell's lists live: reads `GET /events` while mounted, refreshes the
 * cached answers each event makes stale (the sidebar's unread and needs-approval dots, the
 * Scheduled count, schedules), catches up after a dropped stream, and hands every event to the
 * screens that listen (`useUserEvent`).
 */
export function useEventStream(): void {
  const client = useQueryClient();
  useEffect(() => {
    const controller = new AbortController();
    void watchEvents({
      signal: controller.signal,
      onEvent: (event) => {
        for (const filter of staleKeys(event)) void client.invalidateQueries(filter);
        publish(event);
      },
      onOpen: (again) => {
        if (!again) return;
        for (const filter of CATCH_UP) void client.invalidateQueries(filter);
      },
    });
    return () => controller.abort();
  }, [client]);
}

/** Calls `listener` with each user event while the component is mounted. */
export function useUserEvent(listener: (event: UserEvent) => void): void {
  const current = useRef(listener);
  current.current = listener;
  useEffect(() => onUserEvent((event) => current.current(event)), []);
}
