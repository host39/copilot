// The signed-in user. The token (see `token.ts`) says whether a session exists; `GET /auth/me`
// says whose it is and checks that the server still accepts it. A `401` on any request clears the
// token (see `api/client.ts`), which signs every screen out at once.

import {
  hasRole,
  type SetupStatusResponse,
  type SignInResponse,
  type UserResponse,
} from "@copilot/contracts";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { fetchMe, fetchSetup, logout } from "@web/api/routes/auth";
import { clearToken, getToken, setToken, subscribeToken } from "@web/auth/token";
import { log } from "@web/logging";
import { useCallback, useSyncExternalStore } from "react";

export type Session =
  | { status: "loading" }
  | { status: "signed-out" }
  | { status: "signed-in"; user: UserResponse }
  /** A token exists, but the server could not be asked about it. */
  | { status: "unreachable"; error: unknown; retry: () => void };

const meKey = (token: string | null) => ["auth", "me", token] as const;

/** The current token; components re-render when it changes. */
export function useToken(): string | null {
  return useSyncExternalStore(subscribeToken, getToken, getToken);
}

export function useSession(): Session {
  const token = useToken();
  const me = useQuery({
    queryKey: meKey(token),
    queryFn: ({ signal }) => fetchMe(signal),
    enabled: token !== null,
    staleTime: 5 * 60_000,
  });
  if (token === null) return { status: "signed-out" };
  if (me.data) return { status: "signed-in", user: me.data };
  if (me.isError) return { status: "unreachable", error: me.error, retry: () => void me.refetch() };
  return { status: "loading" };
}

/** The signed-in user, for screens behind `RequireSession`. */
export function useUser(): UserResponse {
  const session = useSession();
  if (session.status !== "signed-in") throw new Error("useUser needs a signed-in session");
  return session.user;
}

/** Whether `user` may administer users (UI only: the server checks every request). */
export function isAdmin(user: UserResponse): boolean {
  return hasRole(user.role, "administrator");
}

/** `GET /auth/setup`, shared by the sign-in, setup, and register screens. */
export function useSetupStatus() {
  return useQuery<SetupStatusResponse>({
    queryKey: ["auth", "setup"],
    queryFn: ({ signal }) => fetchSetup(signal),
    staleTime: 0,
  });
}

/** Stores the session of a register or login answer. */
export function useSignIn(): (response: SignInResponse) => void {
  const client = useQueryClient();
  return useCallback(
    (response: SignInResponse) => {
      client.setQueryData(meKey(response.token), response.user);
      // An account exists now; updating the cache (not refetching) keeps the screen in place.
      client.setQueryData<SetupStatusResponse>(
        ["auth", "setup"],
        (status) => status && { ...status, needsSetup: false }
      );
      setToken(response.token);
    },
    [client]
  );
}

/** Revokes the session on the server, then forgets it and every cached response here. */
export function useSignOut(): () => Promise<void> {
  const client = useQueryClient();
  return useCallback(async () => {
    try {
      await logout();
    } catch (error) {
      // The local sign-out still happens; the session expires on the server.
      log("warn", "web.auth.logout.failed", { reason: String(error) });
    }
    clearToken();
    client.clear();
  }, [client]);
}
