import type { ScheduleResponse } from "@copilot/contracts";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { fetchChat, fetchMessages } from "@web/api/routes/chats";
import { keys } from "@web/api/routes/keys";
import { fetchModels } from "@web/api/routes/models";
import { fetchProject } from "@web/api/routes/projects";
import { deleteSchedule, fetchSchedule } from "@web/api/routes/schedules";
import Button from "@web/components/Button/Button";
import { ConfirmDialog } from "@web/components/Dialog/ConfirmDialog";
import { PageTitle } from "@web/components/Layout/PageTitle";
import {
  AssistantBadge,
  EnabledSwitch,
  RunList,
  RunNowButton,
  ScheduleState,
  useScheduleSaved,
} from "@web/components/Schedule/controls";
import { ScheduleForm } from "@web/components/Schedule/ScheduleForm";
import { QueryState } from "@web/components/Settings/QueryState";
import path, { link } from "@web/routes/path";
import { CHAT_PARAM, PROJECT_PARAM } from "@web/routes/schedule/draft";
import { scheduleValues, startValues } from "@web/routes/schedule/rules";
import { useState } from "react";
import { LuArrowLeft, LuTrash2 } from "react-icons/lu";
import { Link, useNavigate, useParams, useSearchParams } from "react-router-dom";

function Back() {
  return (
    <Link to={path.scheduled} className="back-link">
      <LuArrowLeft aria-hidden />
      Scheduled
    </Link>
  );
}

/**
 * The new schedule form, prefilled from a chat (`?chat=`: "Schedule this prompt...") or in a
 * project (`?project=`). Without a model of its own it starts with the project's default model,
 * else the first usable one.
 */
function NewSchedule() {
  const [params] = useSearchParams();
  const navigate = useNavigate();
  const saved = useScheduleSaved();
  const chatId = params.get(CHAT_PARAM);
  const projectId = params.get(PROJECT_PARAM);
  const models = useQuery({ queryKey: keys.models, queryFn: ({ signal }) => fetchModels(signal) });
  const chat = useQuery({
    queryKey: keys.chat(chatId ?? ""),
    queryFn: ({ signal }) => fetchChat(chatId ?? "", signal),
    enabled: chatId !== null,
  });
  const messages = useQuery({
    queryKey: keys.chatMessages(chatId ?? ""),
    queryFn: ({ signal }) => fetchMessages(chatId ?? "", signal),
    enabled: chatId !== null,
  });
  const targetId = chatId !== null ? (chat.data?.projectId ?? null) : projectId;
  const project = useQuery({
    queryKey: keys.project(targetId ?? ""),
    queryFn: ({ signal }) => fetchProject(targetId ?? "", signal),
    enabled: targetId !== null,
  });

  // The form starts once everything it is prefilled with has arrived (or failed).
  const waiting =
    models.isPending ||
    (chatId !== null && (chat.isPending || messages.isPending)) ||
    (targetId !== null && project.isPending);

  const initial = waiting
    ? null
    : startValues({
        chat: chat.data,
        messages: messages.data,
        project: project.data,
        models: models.data ?? [],
      });

  return (
    <section className="screen max-w-2xl">
      <PageTitle page="New Schedule" />
      <Back />
      <h1 className="text-xl font-semibold tracking-tight">New Schedule</h1>
      <p className="mt-1 text-sm text-muted">
        Runs a prompt at set times. Each run opens a new chat under Scheduled.
      </p>
      <div className="mt-6">
        <QueryState query={{ ...models, isPending: waiting }}>
          {initial ? (
            <ScheduleForm
              initial={initial}
              onSaved={(schedule) => {
                saved(schedule);
                navigate(link.schedule(schedule.id), { replace: true });
              }}
            />
          ) : null}
        </QueryState>
      </div>
    </section>
  );
}

/** A saved schedule: its state, on or off, Run now, the form, its runs, and Delete. */
function EditSchedule({ schedule }: { schedule: ScheduleResponse }) {
  const navigate = useNavigate();
  const client = useQueryClient();
  const saved = useScheduleSaved();
  const [confirming, setConfirming] = useState(false);
  const remove = useMutation({
    mutationFn: () => deleteSchedule(schedule.id),
    onSuccess: () => {
      client.removeQueries({ queryKey: keys.schedule(schedule.id) });
      void client.invalidateQueries({ queryKey: [...keys.schedules, "list"] });
      navigate(path.scheduled, { replace: true });
    },
  });

  return (
    <>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h1 className="flex flex-wrap items-center gap-2 text-xl font-semibold tracking-tight">
            <span className="break-words">{schedule.name}</span>
            <AssistantBadge schedule={schedule} />
          </h1>
          <p className="mt-1 text-sm text-muted">{schedule.description}</p>
        </div>
        <EnabledSwitch schedule={schedule} />
      </div>
      <div className="mt-3 space-y-3">
        <ScheduleState schedule={schedule} />
        <div className="flex flex-wrap gap-2">
          <RunNowButton schedule={schedule} />
          <Button
            variant="secondary"
            onClick={() => {
              remove.reset();
              setConfirming(true);
            }}
          >
            <LuTrash2 aria-hidden />
            Delete
          </Button>
        </div>
      </div>

      <section aria-labelledby="schedule-settings" className="card-panel mt-6">
        <h2 id="schedule-settings" className="mb-4 text-base font-semibold">
          Settings
        </h2>
        <ScheduleForm
          key={schedule.updatedAt}
          initial={scheduleValues(schedule)}
          schedule={schedule}
          onSaved={saved}
        />
      </section>

      <section aria-labelledby="schedule-runs" className="mt-6">
        <h2 id="schedule-runs" className="mb-2 text-base font-semibold">
          Runs
        </h2>
        <div className="list-panel py-1">
          <RunList scheduleId={schedule.id} />
        </div>
      </section>

      <ConfirmDialog
        open={confirming}
        title="Delete Schedule?"
        confirm="Delete schedule"
        busy={remove.isPending}
        error={remove.error ? describeError(remove.error).message : undefined}
        onConfirm={() => remove.mutate()}
        onCancel={() => setConfirming(false)}
      >
        <p>{`"${schedule.name}" stops running. The chats of its runs stay.`}</p>
      </ConfirmDialog>
    </>
  );
}

function ScheduleDetails({ scheduleId }: { scheduleId: string }) {
  const schedule = useQuery({
    queryKey: keys.schedule(scheduleId),
    queryFn: ({ signal }) => fetchSchedule(scheduleId, signal),
  });
  return (
    <section className="screen max-w-2xl">
      <PageTitle page="Schedule" />
      <Back />
      <QueryState query={schedule} back={{ to: path.scheduled, label: "Scheduled" }}>
        {schedule.data ? <EditSchedule schedule={schedule.data} /> : null}
      </QueryState>
    </section>
  );
}

/** `/scheduled/new` creates a schedule; `/scheduled/:scheduleId` shows and edits one. */
export default function SchedulePage() {
  const { scheduleId } = useParams();
  return scheduleId ? (
    <ScheduleDetails key={scheduleId} scheduleId={scheduleId} />
  ) : (
    <NewSchedule />
  );
}
