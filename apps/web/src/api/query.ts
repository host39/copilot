import { QueryClient } from "@tanstack/react-query";
import { ApiError } from "@web/api/client";

/**
 * Whether a failed query runs again: never for answers the server gave on purpose (4xx, such as
 * 401, 403, 404, and 429), at most twice more for unreachable servers and 5xx.
 */
export function shouldRetry(failures: number, error: unknown): boolean {
  if (error instanceof ApiError && error.status >= 400 && error.status < 500) return false;
  return failures < 2;
}

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: { retry: shouldRetry, staleTime: 30_000 },
    mutations: { retry: false },
  },
});
