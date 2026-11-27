// Platform behavior: the desktop shortcuts, what the Android back button does, and the chat list
// kept beneath a deep link on phones. Pure functions, so tests check every rule without a browser.

/** The shell's shortcuts (Esc stops an answer on the chat page itself). */
export type Shortcut = "newChat" | "search" | "help";

/** The shortcut list (Ctrl+/), in the spec's words. */
export const SHORTCUTS: readonly { keys: string; action: string }[] = [
  { keys: "Ctrl+Shift+O", action: "New chat" },
  { keys: "Ctrl+K", action: "Search" },
  { keys: "Esc", action: "Stop the answer being written" },
  { keys: "Ctrl+/", action: "Show these shortcuts" },
  { keys: "Enter", action: "Send the message (Shift+Enter adds a line)" },
];

type Keys = Pick<
  KeyboardEvent,
  | "key"
  | "code"
  | "ctrlKey"
  | "metaKey"
  | "shiftKey"
  | "altKey"
  | "isComposing"
  | "defaultPrevented"
>;

/**
 * The shortcut a key press names, or `null`. Ctrl (or Cmd on a Mac) with Shift+O, K, or `/`;
 * never while an input method composes or once something else took the key. `/` counts with or
 * without Shift, since some layouts need Shift to type it.
 */
export function shortcut(event: Keys): Shortcut | null {
  if (event.defaultPrevented || event.isComposing || event.altKey) return null;
  if (!(event.ctrlKey || event.metaKey)) return null;
  const key = event.key.toLowerCase();
  if (event.shiftKey && (key === "o" || event.code === "KeyO")) return "newChat";
  if (!event.shiftKey && key === "k") return "search";
  if (key === "/") return "help";
  return null;
}

/** What the Android back button does next. */
export type BackAction = "close" | "back" | "home" | "exit";

/**
 * Android back: closes an open overlay (drawer, sheet, dialog) first, then goes back in the app's
 * history, then to the chat list, and leaves the app only from there.
 */
export function backAction(state: {
  overlay: boolean;
  canGoBack: boolean;
  atHome: boolean;
}): BackAction {
  if (state.overlay) return "close";
  if (state.canGoBack) return "back";
  return state.atHome ? "exit" : "home";
}

/** Phones and other screens where the sidebar is a drawer (below Tailwind's `lg`). */
export const NARROW_QUERY = "(max-width: 63.99rem)";

/**
 * Whether to put the chat list beneath the page the app opened on: on a narrow screen, when the
 * app's history starts at a page other than the chat list (a link to a chat), so back reaches the
 * chat list before it leaves the app.
 */
export function needsHome(state: { index: number; atHome: boolean; narrow: boolean }): boolean {
  return state.narrow && state.index === 0 && !state.atHome;
}

type Closable = {
  close: () => void;
  dispatchEvent: (event: Event) => boolean;
  requestClose?: () => void;
};

/**
 * Closes the topmost open dialog the way Escape does (its `cancel` event may keep it open), and
 * returns whether there was one.
 */
export function closeTop(dialogs: ArrayLike<Closable>): boolean {
  const top = dialogs[dialogs.length - 1];
  if (!top) return false;
  if (typeof top.requestClose === "function") top.requestClose();
  else if (top.dispatchEvent(new Event("cancel", { cancelable: true }))) top.close();
  return true;
}
