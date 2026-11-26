// Route guards for the table in `routes/index.tsx`.

import { isAdmin, useSession } from "@web/auth/session";
import { ErrorView } from "@web/components/Error/ErrorView";
import { DefaultLoading } from "@web/components/Suspense/DefaultLoading";
import { isNative, serverUrl } from "@web/config";
import NotFoundPage from "@web/routes/not-found/page";
import path from "@web/routes/path";
import { Navigate, Outlet, useLocation } from "react-router-dom";

/** Where a guard sent the user from, so signing in can return there. */
export type FromState = { from?: string };

/** Native builds choose their server first. */
export function RequireServer() {
  if (isNative() && serverUrl() === undefined) return <Navigate to={path.connect} replace />;
  return <Outlet />;
}

/** Screens for signed-in users; everyone else goes to sign-in. */
export function RequireSession() {
  const session = useSession();
  const location = useLocation();

  switch (session.status) {
    case "signed-in":
      return <Outlet />;
    case "signed-out": {
      const state: FromState = { from: `${location.pathname}${location.search}` };
      return <Navigate to={path.login} replace state={state} />;
    }
    case "unreachable":
      return (
        <ErrorView
          title="Cannot reach the server"
          description="Check your connection, then try again."
          onRetry={session.retry}
        />
      );
    default:
      return <DefaultLoading />;
  }
}

/** Sign-in, setup, and register: a signed-in user goes on to where they were headed. */
export function GuestOnly() {
  const session = useSession();
  const location = useLocation();
  const from = (location.state as FromState | null)?.from;

  if (session.status === "signed-in") return <Navigate to={from ?? path.home} replace />;
  if (session.status === "loading") return <DefaultLoading />;
  return <Outlet />;
}

/** Administrator screens (Settings, Users); others see "not found", as the server answers. */
export function RequireAdmin() {
  const session = useSession();
  if (session.status === "signed-in" && isAdmin(session.user)) return <Outlet />;
  return <NotFoundPage />;
}
