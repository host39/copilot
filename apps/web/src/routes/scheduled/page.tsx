import type { ScheduleResponse } from "@copilot/contracts";
import { useQuery } from "@tanstack/react-query";
import { fetchChats } from "@web/api/routes/chats";
import { keys } from "@web/api/routes/keys";
import { fetchProjects } from "@web/api/routes/projects";
import { fetchSchedules } from "@web/api/routes/schedules";
import { PageTitle } from "@web/components/Layout/PageTitle";
import {
  AssistantBadge,
  EnabledSwitch,
  RunList,
  RunNowButton,
  ScheduleState,
} from "@web/components/Schedule/controls";
import { QueryState } from "@web/components/Settings/QueryState";
import { link } from "@web/routes/path";
import { newScheduleLink } from "@web/routes/schedule/draft";
import { targetLabel, unreadRuns } from "@web/routes/schedule/rules";
import { useState } from "react";
import { LuPencil, LuPlus } from "react-icons/lu";
import { Link } from "react-router-dom";

/** Schedule runs read for the unread counts, as the sidebar does. */
const RUNS_READ = 100;

function Runs({ schedule, unread }: { schedule: ScheduleResponse; unread: number }) {
  // The runs load once the list is opened.
  const [open, setOpen] = useState(false);
  return (
    <details
      className="rounded-lg border border-line"
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary className="flex min-h-10 cursor-pointer items-center gap-2 px-3 text-sm">
        Runs
        {unread > 0 ? (
          <span className="count-badge">
            {unread}
            <span className="sr-only"> unread</span>
          </span>
        ) : null}
      </summary>
      <div className="border-t border-line py-1">
        {open ? <RunList scheduleId={schedule.id} /> : null}
      </div>
    </details>
  );
}

function Card({
  schedule,
  target,
  unread,
}: {
  schedule: ScheduleResponse;
  target: string;
  unread: number;
}) {
  return (
    <li className="card-panel space-y-3 p-4 sm:p-5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="flex flex-wrap items-center gap-2 text-base font-semibold">
            <Link to={link.schedule(schedule.id)} className="break-words hover:underline">
              {schedule.name}
            </Link>
            <AssistantBadge schedule={schedule} />
          </h2>
          <p className="text-sm text-muted">{`${target} · ${schedule.description}`}</p>
        </div>
        <EnabledSwitch schedule={schedule} />
      </div>
      <ScheduleState schedule={schedule} />
      <div className="flex flex-wrap items-start gap-2">
        <RunNowButton schedule={schedule} />
        <Link to={link.schedule(schedule.id)} className="icon-button w-auto gap-2 px-3 text-sm">
          <LuPencil aria-hidden />
          Edit
        </Link>
      </div>
      <Runs schedule={schedule} unread={unread} />
    </li>
  );
}

/**
 * Every schedule: name, target, the rule in words, the next run in local time, the last run's
 * status (a pause shows its reason in red), the on switch, Run now, and the runs with their
 * unread counts.
 */
export default function ScheduledPage() {
  const schedules = useQuery({
    queryKey: keys.scheduleList({}),
    queryFn: ({ signal }) => fetchSchedules({}, signal),
  });
  const projects = useQuery({
    queryKey: keys.projectList({}),
    queryFn: ({ signal }) => fetchProjects({}, signal),
  });
  const runs = useQuery({
    queryKey: keys.runs,
    queryFn: ({ signal }) => fetchChats({ scheduled: true, limit: RUNS_READ }, signal),
  });
  const unread = unreadRuns(runs.data?.items ?? []);
  const list = schedules.data ?? [];

  return (
    <section className="screen max-w-3xl">
      <PageTitle page="Scheduled" />
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">Scheduled</h1>
          <p className="mt-1 text-sm text-muted">
            Prompts that run at set times, in a project or on their own.
          </p>
        </div>
        <Link to={newScheduleLink()} className="button-link">
          <LuPlus aria-hidden />
          New schedule
        </Link>
      </div>
      <div className="mt-6">
        <QueryState query={schedules}>
          {list.length === 0 ? (
            <div className="card-panel text-center">
              <p className="text-sm font-medium">No schedules yet</p>
              <p className="mt-1 text-sm text-muted">
                A schedule runs a prompt every hour, day, week, or month, for example a morning news
                summary. Start one here, from a project, or with "Schedule this prompt" in a chat's
                menu.
              </p>
            </div>
          ) : (
            <ul className="space-y-3">
              {list.map((schedule) => (
                <Card
                  key={schedule.id}
                  schedule={schedule}
                  target={targetLabel(schedule.projectId, projects.data)}
                  unread={unread.get(schedule.id) ?? 0}
                />
              ))}
            </ul>
          )}
        </QueryState>
      </div>
    </section>
  );
}
