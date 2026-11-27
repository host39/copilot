import type {
  ModelResponse,
  OnAsk,
  ProjectResponse,
  ReasoningLevel,
  ScheduleResponse,
} from "@copilot/contracts";
import { useMutation, useQuery } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { keys } from "@web/api/routes/keys";
import { fetchModels } from "@web/api/routes/models";
import { fetchProjects } from "@web/api/routes/projects";
import { createSchedule, updateSchedule } from "@web/api/routes/schedules";
import { fetchToolServers } from "@web/api/routes/tool-servers";
import Button from "@web/components/Button/Button";
import { FormAlert } from "@web/components/Form/FormAlert";
import { Select } from "@web/components/Form/Select";
import { TextArea } from "@web/components/Form/TextArea";
import { TextField } from "@web/components/Form/TextField";
import { groupModels, levelLabel, reasoningLevels } from "@web/components/ModelPicker/models";
import { RuleFields } from "@web/components/Schedule/RuleFields";
import { projectLabel } from "@web/routes/project/forms";
import {
  checkValues,
  createBody,
  NAME_MAX_CHARS,
  type ScheduleValues,
  targetLabel,
  targetTools,
  toolNames,
  updateBody,
} from "@web/routes/schedule/rules";
import { useState } from "react";

const ON_ASK: readonly { value: OnAsk; label: string; hint: string }[] = [
  {
    value: "deny",
    label: "Deny and continue",
    hint: "A tool that asks is denied, and the answer goes on without it.",
  },
  {
    value: "pause",
    label: "Pause until I approve",
    hint: "The run waits in its chat until you approve or deny the call.",
  },
];

/** "Deny and continue" or "Pause until I approve", with the target's tools that ask. */
function OnAskPicker({
  value,
  onChange,
  askTools,
}: {
  value: OnAsk;
  onChange: (value: OnAsk) => void;
  askTools: string;
}) {
  return (
    <fieldset>
      <legend className="field-label">When a tool asks for approval</legend>
      <p className="field-hint">
        {askTools
          ? `Tools that ask: ${askTools}.`
          : "No tool of this project asks for approval now."}
      </p>
      <div className="mt-2 grid gap-2 sm:grid-cols-2">
        {ON_ASK.map((option) => (
          <label key={option.value} className="choice-card flex-col gap-0.5">
            <input
              type="radio"
              name="onAsk"
              value={option.value}
              checked={value === option.value}
              onChange={() => onChange(option.value)}
              className="sr-only"
            />
            <span className="font-medium">{option.label}</span>
            <span className="text-muted">{option.hint}</span>
          </label>
        ))}
      </div>
    </fieldset>
  );
}

/** The project a new schedule runs in, or standalone. */
function TargetField({
  projects,
  projectId,
  error,
  onChange,
}: {
  projects: ProjectResponse[] | undefined;
  projectId: string | null;
  error: string | undefined;
  onChange: (projectId: string | null, defaultModelId: string | null) => void;
}) {
  return (
    <Select
      label="Runs in"
      value={projectId ?? ""}
      error={error}
      onChange={(event) => {
        const next = projects?.find((item) => item.id === event.target.value);
        onChange(next?.id ?? null, next?.defaultModelId ?? null);
      }}
      hint={
        projectId
          ? "Runs use the project's instructions, files, and tools."
          : "Runs use only the instructions below: no files and no tools."
      }
    >
      <option value="">Standalone</option>
      {(projects ?? []).map((item) => (
        <option key={item.id} value={item.id}>
          {projectLabel(item)}
        </option>
      ))}
    </Select>
  );
}

/** The model a schedule runs with and, for reasoning models, its level. */
function ModelFields({
  models,
  values,
  errors,
  onChange,
}: {
  models: ModelResponse[] | undefined;
  values: Pick<ScheduleValues, "modelId" | "reasoning" | "projectId">;
  errors: Record<string, string>;
  onChange: (modelId: string, reasoning: ReasoningLevel | null) => void;
}) {
  const model = models?.find((item) => item.id === values.modelId);
  const levels = model ? reasoningLevels(model) : [];
  const missing = values.modelId !== "" && models !== undefined && !model;
  return (
    <div className="grid gap-4 sm:grid-cols-2">
      <Select
        label="Model"
        value={values.modelId}
        error={errors.modelId}
        disabled={models === undefined}
        onChange={(event) => onChange(event.target.value, values.reasoning)}
        hint={
          missing ? "This model is turned off or no longer listed. Choose another one." : undefined
        }
      >
        <option value="">Choose a model</option>
        {missing ? <option value={values.modelId}>Unavailable model</option> : null}
        {groupModels(models ?? []).map((group) => (
          <optgroup key={group.providerId} label={group.provider}>
            {group.models.map((item) => (
              <option key={item.id} value={item.id}>
                {item.displayName}
              </option>
            ))}
          </optgroup>
        ))}
      </Select>
      {model && levels.length > 0 ? (
        <Select
          label="Reasoning"
          value={values.reasoning ?? ""}
          error={errors.reasoning}
          onChange={(event) =>
            onChange(values.modelId, (event.target.value || null) as ReasoningLevel | null)
          }
        >
          <option value="">{values.projectId ? "Project default" : levelLabel(null, model)}</option>
          {levels.map((level) => (
            <option key={level} value={level}>
              {levelLabel(level)}
            </option>
          ))}
        </Select>
      ) : null}
    </div>
  );
}

type ScheduleFormProps = {
  initial: ScheduleValues;
  /** The saved schedule when editing; its target is fixed. */
  schedule?: ScheduleResponse;
  onSaved: (schedule: ScheduleResponse) => void;
};

/**
 * A schedule's settings: name, target (a project or standalone, chosen when created), model and
 * reasoning, when it runs with the preview of its next runs, the prompt, the instructions of a
 * standalone schedule, and what a run does when a tool asks. Checks core's rules before sending
 * and puts the server's answer next to the field it names.
 */
export function ScheduleForm({ initial, schedule, onSaved }: ScheduleFormProps) {
  const [values, setValues] = useState(initial);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const models = useQuery({ queryKey: keys.models, queryFn: ({ signal }) => fetchModels(signal) });
  const projects = useQuery({
    queryKey: keys.projectList({}),
    queryFn: ({ signal }) => fetchProjects({}, signal),
  });
  const servers = useQuery({
    queryKey: keys.toolServers,
    queryFn: ({ signal }) => fetchToolServers(signal),
  });
  const save = useMutation({
    mutationFn: async (checked: { values: ScheduleValues; cron: string }) => {
      if (!schedule) return createSchedule(createBody(checked.values, checked.cron));
      const body = updateBody(schedule, checked.values, checked.cron);
      return Object.keys(body).length === 0 ? schedule : updateSchedule(schedule.id, body);
    },
    onSuccess: onSaved,
    onError: (failure) => {
      const problem = describeError(failure);
      if (problem.field) setErrors({ [problem.field]: problem.message });
    },
  });

  const set = (change: Partial<ScheduleValues>) =>
    setValues((current) => ({ ...current, ...change }));
  const project = projects.data?.find((item) => item.id === values.projectId) ?? null;
  const askTools = toolNames(
    targetTools(project, servers.data).filter((tool) => tool.mode === "ask")
  );
  const problem = save.error ? describeError(save.error) : null;

  return (
    <form
      noValidate
      className="space-y-5"
      onSubmit={(event) => {
        event.preventDefault();
        const checked = checkValues(values);
        if (!checked.ok) {
          setErrors(checked.errors);
          return;
        }
        setErrors({});
        save.mutate(checked);
      }}
    >
      <TextField
        label="Name"
        value={values.name}
        maxLength={NAME_MAX_CHARS}
        error={errors.name}
        onChange={(event) => set({ name: event.target.value })}
        hint="Each run opens a chat named after it, with the date."
      />
      {schedule ? (
        <div>
          <p className="field-label">Runs in</p>
          <p className="mt-1.5 text-sm">{targetLabel(schedule.projectId, projects.data)}</p>
        </div>
      ) : (
        <TargetField
          projects={projects.data}
          projectId={values.projectId}
          error={errors.projectId}
          onChange={(projectId, defaultModelId) =>
            set({ projectId, modelId: values.modelId || defaultModelId || "" })
          }
        />
      )}
      <ModelFields
        models={models.data}
        values={values}
        errors={errors}
        onChange={(modelId, reasoning) => set({ modelId, reasoning })}
      />
      <RuleFields
        rule={values.rule}
        timezone={values.timezone}
        errors={errors}
        onRule={(rule) => set({ rule })}
        onTimezone={(timezone) => set({ timezone })}
      />
      <TextArea
        label="Prompt"
        value={values.prompt}
        error={errors.prompt}
        rows={5}
        onChange={(event) => set({ prompt: event.target.value })}
        hint="Sent as the first message of each run's chat. Runs are unattended."
      />
      {values.projectId === null ? (
        <TextArea
          label="Instructions"
          value={values.instructions}
          error={errors.instructions}
          rows={4}
          onChange={(event) => set({ instructions: event.target.value })}
          hint="The system prompt of each run. Optional."
        />
      ) : (
        <OnAskPicker
          value={values.onAsk}
          onChange={(onAsk) => set({ onAsk })}
          askTools={askTools}
        />
      )}
      <FormAlert>{problem && !problem.field ? problem.message : null}</FormAlert>
      <Button type="submit" busy={save.isPending}>
        {schedule ? "Save" : "Create schedule"}
      </Button>
    </form>
  );
}
