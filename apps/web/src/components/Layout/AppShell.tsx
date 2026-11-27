import { type Shell, ShellContext } from "@web/components/Layout/Shell";
import { Sidebar } from "@web/components/Sidebar/Sidebar";
import { PageLoading } from "@web/components/Suspense/PageLoading";
import { useEventStream } from "@web/hooks/useEvents";
import { Suspense, useEffect, useMemo, useRef, useState } from "react";
import { LuMenu, LuX } from "react-icons/lu";
import { Outlet, useLocation } from "react-router-dom";

/**
 * The signed-in layout: a sidebar beside the screen on wide windows, and a drawer behind a menu
 * button on phones (a modal `<dialog>`, so Escape, focus, and the inert page come from the
 * browser). A page with its own top bar (`useOwnHeader`) replaces the phone header's row. The
 * shell reads the user's live events (`useEventStream`), so lists and dots stay current.
 */
export function AppShell() {
  useEventStream();
  const drawer = useRef<HTMLDialogElement>(null);
  const [open, setOpen] = useState(false);
  const [ownHeader, setOwnHeader] = useState(false);
  const location = useLocation();
  const shell = useMemo<Shell>(
    () => ({
      openMenu: () => {
        setOpen(true);
        drawer.current?.showModal();
      },
      setOwnHeader,
    }),
    []
  );

  // Following a link closes the drawer.
  // biome-ignore lint/correctness/useExhaustiveDependencies: runs on every navigation
  useEffect(() => {
    drawer.current?.close();
  }, [location.key]);

  return (
    <ShellContext value={shell}>
      <div className="flex h-dvh bg-canvas text-fg">
        <aside className="hidden w-72 shrink-0 border-r border-line lg:block">
          <Sidebar />
        </aside>

        {/* biome-ignore lint/a11y/useKeyWithClickEvents: a backdrop tap is the pointer shortcut; keyboards close with Escape (native) or the Close button */}
        <dialog
          ref={drawer}
          id="drawer"
          aria-label="Menu"
          onClose={() => setOpen(false)}
          onClick={(event) => {
            if (event.target === event.currentTarget) drawer.current?.close();
          }}
          className="m-0 h-dvh max-h-none w-[min(20rem,85vw)] max-w-none border-r border-line bg-surface p-0 text-fg backdrop:bg-black/50 lg:hidden"
        >
          <div className="relative h-full">
            <button
              type="button"
              onClick={() => drawer.current?.close()}
              aria-label="Close menu"
              className="absolute top-3 right-3 z-10 flex size-10 items-center justify-center rounded-lg text-muted hover:bg-raised hover:text-fg"
            >
              <LuX aria-hidden />
            </button>
            {open ? <Sidebar /> : null}
          </div>
        </dialog>

        <div className="flex min-w-0 flex-1 flex-col">
          <header
            className={`h-14 shrink-0 items-center gap-2 border-b border-line px-2 lg:hidden ${ownHeader ? "hidden" : "flex"}`}
          >
            <button
              type="button"
              onClick={shell.openMenu}
              aria-label="Open menu"
              aria-controls="drawer"
              aria-expanded={open}
              className="flex size-10 items-center justify-center rounded-lg hover:bg-raised"
            >
              <LuMenu aria-hidden className="size-5" />
            </button>
            <img src="/icons/icon.svg" alt="" className="size-6" />
            <span className="font-semibold tracking-tight">Copilot</span>
          </header>
          <main className="min-h-0 flex-1 overflow-y-auto">
            <Suspense fallback={<PageLoading />}>
              <Outlet />
            </Suspense>
          </main>
        </div>
      </div>
    </ShellContext>
  );
}
