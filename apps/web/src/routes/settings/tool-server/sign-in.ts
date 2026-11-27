// OAuth sign-in to a tool server: what its state reads as, opening the sign-in page, and what
// the page a sign-in returns to (`/oauth/done`) says.

import type { SignInState, ToolServerResponse } from "@copilot/contracts";
import type { BadgeTone } from "@web/components/Badge/Badge";

/** How long a started sign-in may take (the server forgets it after 10 minutes). */
export const SIGN_IN_WAIT_MS = 10 * 60 * 1000;

/** How often a native app checks whether the sign-in in the system browser finished. */
export const SIGN_IN_POLL_MS = 3_000;

/** The badge of a sign-in state. */
export function signInBadge(state: SignInState): { label: string; tone: BadgeTone } {
  switch (state) {
    case "signed_in":
      return { label: "Signed in", tone: "ok" };
    case "expired":
      return { label: "Expired: sign in again", tone: "warn" };
    default:
      return { label: "Needs sign-in", tone: "warn" };
  }
}

/** Whether an OAuth server has no usable sign-in. */
export function needsSignIn(server: ToolServerResponse): boolean {
  return server.authMode === "oauth" && server.oauth?.state !== "signed_in";
}

/**
 * Opens the authorization page. The web app leaves for it (the sign-in comes back to this
 * server's `/oauth/done`); in the native apps the navigation guard hands any outside address to
 * the system browser, and the app stays where it is.
 */
export function openSignIn(url: string, target: Pick<Location, "assign"> = window.location): void {
  target.assign(url);
}

/** The results `/api/oauth/callback` sends the browser to `/oauth/done` with. */
export type SignInResult = "signed-in" | "denied" | "expired" | "failed";

/** What `/oauth/done` says for a `result`; anything unknown reads as failed. */
export function doneMessage(result: string | null): { title: string; body: string } {
  switch (result) {
    case "signed-in":
      return {
        title: "Signed in",
        body: "The tool server is signed in. You can close this window and go back to Copilot.",
      };
    case "denied":
      return {
        title: "Sign-in cancelled",
        body: "Access was not granted at the sign-in page. Start again from the tool server's settings to try once more.",
      };
    case "expired":
      return {
        title: "Sign-in expired",
        body: "This sign-in took longer than 10 minutes or was already used. Start it again from the tool server's settings.",
      };
    default:
      return {
        title: "Sign-in failed",
        body: "The sign-in did not finish. Start it again from the tool server's settings; if it keeps failing, check the server's address.",
      };
  }
}
