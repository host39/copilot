// What the Scheduled list and a schedule's page share: keeping the cache in step after a change,
// the enabled switch, Run now, the assistant badge, the status line, and the runs.

import type { ScheduleResponse } from "@copilot/contracts";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { keys } from "@web/api/routes/keys";
import { runSchedule, updateSchedule } from "@web/api/routes/schedules";
import { Badge } from "@web/components/Badge/Badge";
import Button from "@web/components/Button/Button";
import { ChatRow } from "@web/components/Chats/ChatRow";
import { useChatPages } from "@web/components/Chats/pages";
import { Switch } from "@web/components/Form/Switch";
import { link } from "@web/routes/path";
import { formatRun, scheduleStatus } from "@web/routes/schedule/rules";
import { LuBot, LuPlay } from "react-icons/lu";
import { useNavigate } from "react-router-dom";

/** Stores a saved schedule in the cache and refreshes the lists that show it. */
export function useScheduleSaved() {
  const client = useQueryClient();
  return (schedule: ScheduleResponse) => {
    client.setQueryData(keys.schedule(schedule.id), schedule);
    void client.invalidateQueries({ queryKey: [...keys.schedules, "list"] });
  };
}

/** "By assistant" for a schedule the assistant created with the `schedule_prompt` tool. */
export function AssistantBadge({ schedule }: { schedule: Pick<ScheduleResponse, "createdBy"> }) {
  if (schedule.createdBy !== "assistant") return null;
  return (
    <Badge tone="accent">
      <LuBot aria-hidden />
      By assistant
    </Badge>
  );
}

/** The next run in local time (or why there is none) and the last run's status. */
export function ScheduleState({ schedule }: { schedule: ScheduleResponse }) {
  const status = scheduleStatus(schedule);
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-sm">
      <span className="text-muted">
        {schedule.nextRunAt ? (
          <>
            {"Next: "}
            <time dateTime={schedule.nextRunAt}>{formatRun(schedule.nextRunAt)}</time>
          </>
        ) : (
          "No next run"
        )}
      </span>
      <Badge tone={status.tone}>{status.label}</Badge>
      {status.detail ? (
        <span className={status.tone === "danger" ? "text-danger" : "text-muted"}>
          {status.detail}
        </span>
      ) : null}
    </div>
  );
}

/** Turns a schedule on or off; the server's reason shows when it refuses (20 turned on). */
export function EnabledSwitch({ schedule }: { schedule: ScheduleResponse }) {
  const saved = useScheduleSaved();
  const toggle = useMutation({
    mutationFn: (enabled: boolean) => updateSchedule(schedule.id, { enabled }),
    onSuccess: saved,
  });
  return (
    <div>
      <Switch
        label={`${schedule.name} turned on`}
        hideLabel
        checked={toggle.isPending ? (toggle.variables ?? schedule.enabled) : schedule.enabled}
        disabled={toggle.isPending}
        onChange={(event) => toggle.mutate(event.target.checked)}
      />
      {toggle.isError ? (
        <p role="alert" className="field-error">
          {describeError(toggle.error).message}
        </p>
      ) : null}
    </div>
  );
}

/**
 * Run now: runs the schedule unattended, like a scheduled fire, and opens the run's chat, which
 * follows the answer.
 */
export function RunNowButton({ schedule }: { schedule: ScheduleResponse }) {
  const navigate = useNavigate();
  const client = useQueryClient();
  const run = useMutation({
    mutationFn: () => runSchedule(schedule.id),
    onSuccess: (chatId) => {
      void client.invalidateQueries({ queryKey: keys.runs });
      void client.invalidateQueries({ queryKey: keys.schedules });
      navigate(link.chat(chatId));
    },
  });
  return (
    <div>
      <Button variant="secondary" busy={run.isPending} onClick={() => run.mutate()}>
        <LuPlay aria-hidden />
        Run now
      </Button>
      {run.isError ? (
        <p role="alert" className="field-error">
          {describeError(run.error).message}
        </p>
      ) : null}
    </div>
  );
}

/** The chats a schedule's runs created, newest first, with their unread dots. */
export function RunList({ scheduleId }: { scheduleId: string }) {
  const pages = useChatPages({ scheduleId });
  if (pages.isPending) return <p className="px-3 py-2 text-sm text-muted">Loading runs...</p>;
  if (pages.isError) {
    return (
      <p role="alert" className="px-3 py-2 text-sm text-danger">
        {describeError(pages.error).message}
      </p>
    );
  }
  if (pages.chats.length === 0) {
    return <p className="px-3 py-2 text-sm text-muted">No runs yet.</p>;
  }
  return (
    <div>
      <ul>
        {pages.chats.map((chat) => (
          <li key={chat.id}>
            <ChatRow chat={chat} />
          </li>
        ))}
      </ul>
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
    </div>
  );
}
