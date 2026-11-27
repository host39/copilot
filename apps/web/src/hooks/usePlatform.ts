import {
  backAction,
  closeTop,
  NARROW_QUERY,
  needsHome,
  type Shortcut,
  shortcut,
} from "@web/components/Layout/platform";
import { isNative } from "@web/config";
import { log } from "@web/logging";
import path from "@web/routes/path";
import { useEffect, useRef } from "react";
import { useLocation, useNavigate } from "react-router-dom";

/** Where the router stands in this tab's history of the app (0 on the page it opened on). */
function historyIndex(): number {
  const state = window.history.state as { idx?: unknown } | null;
  return typeof state?.idx === "number" ? state.idx : 0;
}

/** The open dialogs, the topmost last (the drawer, sheets, and confirmations are all `<dialog>`s). */
function openDialogs(): NodeListOf<HTMLDialogElement> {
  return document.querySelectorAll<HTMLDialogElement>("dialog[open]");
}

/**
 * The desktop shortcuts: Ctrl+Shift+O opens a new chat, Ctrl+K search (or its field, when search
 * is open), and Ctrl+/ the shortcut list. They wait while a dialog is open.
 */
export function useShortcuts(help: () => void): void {
  const navigate = useNavigate();
  const location = useLocation();
  const at = useRef(location.pathname);
  at.current = location.pathname;
  const helper = useRef(help);
  helper.current = help;

  useEffect(() => {
    const actions: Record<Shortcut, () => void> = {
      newChat: () => navigate(path.home),
      search: () => {
        if (at.current === path.search) document.getElementById("search")?.focus();
        else navigate(path.search);
      },
      help: () => helper.current(),
    };
    const listener = (event: KeyboardEvent) => {
      const name = shortcut(event);
      if (!name || openDialogs().length > 0) return;
      event.preventDefault();
      actions[name]();
    };
    document.addEventListener("keydown", listener);
    return () => document.removeEventListener("keydown", listener);
  }, [navigate]);
}

/**
 * Android back in the native app: closes the topmost dialog, then goes back in the app's history,
 * then to the chat list, and leaves the app only from there. Browsers already close modal dialogs
 * on back (close requests) and go back in history themselves.
 */
export function useBackButton(): void {
  const navigate = useNavigate();
  const location = useLocation();
  const at = useRef(location.pathname);
  at.current = location.pathname;

  useEffect(() => {
    if (!isNative()) return;
    let stopped = false;
    let unregister: (() => Promise<void>) | undefined;
    import("@tauri-apps/api/app")
      .then(async (app) => {
        const listener = await app.onBackButtonPress(() => {
          const action = backAction({
            overlay: openDialogs().length > 0,
            canGoBack: historyIndex() > 0,
            atHome: at.current === path.home,
          });
          if (action === "close") closeTop(openDialogs());
          else if (action === "back") navigate(-1);
          else if (action === "home") navigate(path.home, { replace: true });
          else void app.exit(0);
        });
        if (stopped) void listener.unregister();
        else unregister = () => listener.unregister();
      })
      .catch((error: unknown) => log("warn", "web.back.listen.failed", { error: String(error) }));
    return () => {
      stopped = true;
      void unregister?.();
    };
  }, [navigate]);
}

/**
 * On a narrow screen, an app opened on a page other than the chat list (a link to a chat) gets the
 * chat list beneath it, so back reaches the list before it leaves the app.
 */
export function useHomeBeneath(): void {
  const navigate = useNavigate();
  const location = useLocation();
  const done = useRef(false);

  useEffect(() => {
    if (done.current) return;
    done.current = true;
    const narrow = window.matchMedia?.(NARROW_QUERY).matches ?? false;
    const atHome = location.pathname === path.home;
    if (!needsHome({ index: historyIndex(), atHome, narrow })) return;
    const target = `${location.pathname}${location.search}${location.hash}`;
    navigate(path.home, { replace: true });
    navigate(target, { state: location.state });
  }, [location, navigate]);
}
