import type { ProjectResponse } from "@copilot/contracts";
import { useQuery } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { fetchChats } from "@web/api/routes/chats";
import { keys } from "@web/api/routes/keys";
import { fetchProjects } from "@web/api/routes/projects";
import { useSignOut, useUser } from "@web/auth/session";
import { ChatRow } from "@web/components/Chats/ChatRow";
import { useChatPages } from "@web/components/Chats/pages";
import { groupByDay, unreadBadge } from "@web/components/Sidebar/groups";
import { newChatLink } from "@web/routes/home/draft";
import path, { link } from "@web/routes/path";
import { type ReactNode, useId } from "react";
import {
  LuCalendarClock,
  LuFolder,
  LuFolders,
  LuLogOut,
  LuPlus,
  LuSearch,
  LuSettings,
  LuSquarePen,
} from "react-icons/lu";
import { Link, NavLink } from "react-router-dom";

/** The sidebar shows the 5 most recently updated projects after the pinned ones. */
const RECENT_PROJECTS = 5;
/** Schedule runs read for the Scheduled entry's unread count. */
const RUNS_READ = 100;

function Item({
  to,
  icon,
  badge,
  children,
}: {
  to: string;
  icon: ReactNode;
  badge?: string | null;
  children: ReactNode;
}) {
  return (
    <NavLink to={to} end className="nav-item">
      <span className="text-muted" aria-hidden>
        {icon}
      </span>
      <span className="min-w-0 flex-1 truncate">{children}</span>
      {badge ? (
        <span className="rounded-full bg-accent px-1.5 text-xs font-medium text-on-accent">
          {badge}
          <span className="sr-only"> unread</span>
        </span>
      ) : null}
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
  const id = useId();
  return (
    <section aria-labelledby={id} className="mt-5">
      <div className="flex min-h-8 items-center justify-between px-3">
        <h2 id={id} className="text-xs font-medium uppercase tracking-wider text-muted">
          {title}
        </h2>
        {action}
      </div>
      <div className="mt-1">{children}</div>
    </section>
  );
}

function ProjectRow({ project }: { project: ProjectResponse }) {
  return (
    <li className="group flex items-center">
      <NavLink to={link.project(project.id)} className="nav-item min-w-0 flex-1">
        <span aria-hidden className="flex w-4 justify-center text-muted">
          {project.icon || <LuFolder />}
        </span>
        <span className="truncate">{project.name}</span>
      </NavLink>
      <Link
        to={newChatLink(project.id)}
        aria-label={`New chat in ${project.name}`}
        title="New chat in this project"
        className="flex size-8 shrink-0 items-center justify-center rounded-lg text-muted hover:bg-raised hover:text-fg"
      >
        <LuPlus aria-hidden />
      </Link>
    </li>
  );
}

function Projects() {
  const pinned = useQuery({
    queryKey: keys.projectList({ pinned: true }),
    queryFn: ({ signal }) => fetchProjects({ pinned: true }, signal),
  });
  const recent = useQuery({
    queryKey: keys.projectList({ pinned: false, limit: RECENT_PROJECTS }),
    queryFn: ({ signal }) => fetchProjects({ pinned: false, limit: RECENT_PROJECTS }, signal),
  });
  const list = [...(pinned.data ?? []), ...(recent.data ?? [])];

  return (
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
      {list.length > 0 ? (
        <ul>
          {list.map((project) => (
            <ProjectRow key={project.id} project={project} />
          ))}
        </ul>
      ) : null}
      <Item to={path.projects} icon={<LuFolders />}>
        All projects
      </Item>
    </Section>
  );
}

function Pinned() {
  const pages = useChatPages({ pinned: true });
  if (pages.chats.length === 0) return null;
  return (
    <Section title="Pinned">
      <ul>
        {pages.chats.map((chat) => (
          <li key={chat.id}>
            <ChatRow chat={chat} />
          </li>
        ))}
      </ul>
    </Section>
  );
}

function Recent() {
  const pages = useChatPages({ scheduled: false, pinned: false });
  const groups = groupByDay(pages.chats, new Date());

  return (
    <Section title="Recent">
      {pages.isError ? (
        <p role="alert" className="px-3 py-2 text-sm text-danger">
          {describeError(pages.error).message}
        </p>
      ) : null}
      {pages.isSuccess && groups.length === 0 ? (
        <p className="px-3 py-2 text-sm text-muted">No chats yet.</p>
      ) : null}
      {groups.map((group) => (
        <div key={group.day} className="mb-2">
          <h3 className="px-3 pt-2 pb-1 text-xs text-muted">{group.label}</h3>
          <ul>
            {group.chats.map((chat) => (
              <li key={chat.id}>
                <ChatRow chat={chat} />
              </li>
            ))}
          </ul>
        </div>
      ))}
      {pages.hasNextPage ? (
        <button
          type="button"
          onClick={() => void pages.fetchNextPage()}
          disabled={pages.isFetchingNextPage}
          className="nav-item w-full text-muted"
        >
          {pages.isFetchingNextPage ? "Loading..." : "Show more"}
        </button>
      ) : null}
    </Section>
  );
}

/**
 * The navigation of the spec's sidebar: New chat, Search, Scheduled with its unread count,
 * Projects (pinned, then the 5 most recent, each with a `+` for a new chat in it), pinned chats,
 * recent chats grouped by day (schedule runs stay under Scheduled), and the user menu.
 */
export function Sidebar() {
  const user = useUser();
  const signOut = useSignOut();
  const runs = useQuery({
    queryKey: keys.runs,
    queryFn: ({ signal }) => fetchChats({ scheduled: true, limit: RUNS_READ }, signal),
  });

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
        <Item
          to={path.scheduled}
          icon={<LuCalendarClock />}
          badge={unreadBadge(runs.data?.items ?? [])}
        >
          Scheduled
        </Item>
        <Projects />
        <Pinned />
        <Recent />
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
