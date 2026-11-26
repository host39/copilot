import { useSignOut, useUser } from "@web/auth/session";
import path from "@web/routes/path";
import type { ReactNode } from "react";
import {
  LuCalendarClock,
  LuFolder,
  LuLogOut,
  LuPlus,
  LuSearch,
  LuSettings,
  LuSquarePen,
} from "react-icons/lu";
import { Link, NavLink } from "react-router-dom";

function Item({ to, icon, children }: { to: string; icon: ReactNode; children: ReactNode }) {
  return (
    <NavLink to={to} end className="nav-item">
      <span className="text-muted" aria-hidden>
        {icon}
      </span>
      {children}
    </NavLink>
  );
}

function Section({
  title,
  action,
  children,
}: {
  title: string;
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="mt-6">
      <div className="flex items-center justify-between px-3">
        <h2 className="text-xs font-medium uppercase tracking-wider text-muted">{title}</h2>
        {action}
      </div>
      <div className="mt-1">{children}</div>
    </section>
  );
}

/**
 * The navigation of the spec's sidebar: New chat, Search, Scheduled, Projects, chats, and the user
 * menu. The project and chat lists arrive with their screens.
 */
export function Sidebar() {
  const user = useUser();
  const signOut = useSignOut();

  return (
    <nav aria-label="Main" className="flex h-full flex-col bg-surface">
      <div className="flex-1 overflow-y-auto px-2 py-3">
        <div className="mb-3 flex items-center gap-2 px-3 py-1">
          <img src="/icons/icon.svg" alt="" className="size-7" />
          <span className="font-semibold tracking-tight">Copilot</span>
        </div>
        <Item to={path.home} icon={<LuSquarePen />}>
          New chat
        </Item>
        <Item to={path.search} icon={<LuSearch />}>
          Search
        </Item>
        <Item to={path.scheduled} icon={<LuCalendarClock />}>
          Scheduled
        </Item>

        <Section
          title="Projects"
          action={
            <Link
              to={path.projectNew}
              aria-label="New project"
              className="flex size-8 items-center justify-center rounded-lg text-muted hover:bg-raised hover:text-fg"
            >
              <LuPlus aria-hidden />
            </Link>
          }
        >
          <Item to={path.projects} icon={<LuFolder />}>
            All projects
          </Item>
        </Section>

        <Section title="Chats">
          <p className="px-3 py-2 text-sm text-muted">No chats yet.</p>
        </Section>
      </div>

      <div className="border-t border-line px-2 py-3">
        <div className="px-3 pb-2">
          <p className="truncate text-sm font-medium">{user.displayName}</p>
          <p className="truncate text-xs text-muted">@{user.username}</p>
        </div>
        <Item to={path.settings} icon={<LuSettings />}>
          Settings
        </Item>
        <button type="button" onClick={() => void signOut()} className="nav-item w-full">
          <span className="text-muted" aria-hidden>
            <LuLogOut />
          </span>
          Sign out
        </button>
      </div>
    </nav>
  );
}
