import { createContext, useContext, useLayoutEffect } from "react";
import { LuMenu } from "react-icons/lu";

export type Shell = {
  /** Opens the drawer (the sidebar on phones). */
  openMenu: () => void;
  /** A page with its own top bar on phones hides the shell's (the chat screen). */
  setOwnHeader: (own: boolean) => void;
};

export const ShellContext = createContext<Shell | null>(null);

/**
 * Called by a page that draws its own top bar with a `MenuButton`: the shell leaves out its bar
 * while the page is shown, so phones keep one row.
 */
export function useOwnHeader(): void {
  const shell = useContext(ShellContext);
  useLayoutEffect(() => {
    if (!shell) return;
    shell.setOwnHeader(true);
    return () => shell.setOwnHeader(false);
  }, [shell]);
}

/** The drawer button of a page's own top bar; hidden on wide windows and outside the shell. */
export function MenuButton() {
  const shell = useContext(ShellContext);
  if (!shell) return null;
  return (
    <button
      type="button"
      onClick={shell.openMenu}
      aria-label="Open menu"
      aria-controls="drawer"
      className="icon-button text-fg lg:hidden"
    >
      <LuMenu aria-hidden className="size-5" />
    </button>
  );
}
