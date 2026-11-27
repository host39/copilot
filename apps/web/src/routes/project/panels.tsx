// The project page's side panel: instructions, default model and reasoning, and tools. Each saves
// only what changed. Files are `components/Project/FilePanel`.

import type {
  BuiltinTool,
  ProjectDetailsResponse,
  ProjectResponse,
  ReasoningLevel,
} from "@copilot/contracts";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { keys } from "@web/api/routes/keys";
import { fetchModels } from "@web/api/routes/models";
import { updateProject } from "@web/api/routes/projects";
import { fetchToolServers } from "@web/api/routes/tool-servers";
import { Badge } from "@web/components/Badge/Badge";
import Button from "@web/components/Button/Button";
import { FormAlert } from "@web/components/Form/FormAlert";
import { FormNotice } from "@web/components/Form/FormNotice";
import { TextArea } from "@web/components/Form/TextArea";
import { groupModels, levelLabel, reasoningLevels } from "@web/components/ModelPicker/models";
import path from "@web/routes/path";
import { instructionsBody, modelBody, toolsBody } from "@web/routes/project/forms";
import { type ReactNode, type SelectHTMLAttributes, useId, useState } from "react";
import { Link } from "react-router-dom";

/** Saves a project change and refreshes the project's queries (the capacity bar follows the model). */
function useSave(project: ProjectResponse) {
  const client = useQueryClient();
  const [saved, setSaved] = useState(false);
  const mutation = useMutation({
    mutationFn: async (body: Parameters<typeof updateProject>[1]) =>
      Object.keys(body).length === 0 ? project : updateProject(project.id, body),
    onMutate: () => setSaved(false),
    onSuccess: () => {
      setSaved(true);
      void client.invalidateQueries({ queryKey: keys.projects });
    },
  });
  return { ...mutation, saved };
}

function Footer({
  busy,
  error,
  saved,
  label = "Save",
}: {
  busy: boolean;
  error: unknown;
  saved: boolean;
  label?: string;
}) {
  return (
    <div className="space-y-2">
      <FormAlert>{error ? describeError(error).message : null}</FormAlert>
      <FormNotice>{saved ? "Saved." : null}</FormNotice>
      <Button type="submit" busy={busy}>
        {label}
      </Button>
    </div>
  );
}

/** The project's system prompt. */
export function InstructionsPanel({ project }: { project: ProjectResponse }) {
  const save = useSave(project);
  return (
    <form
      noValidate
      className="space-y-3"
      onSubmit={(event) => {
        event.preventDefault();
        const text = new FormData(event.currentTarget).get("instructions");
        save.mutate(instructionsBody(project, typeof text === "string" ? text : ""));
      }}
    >
      <TextArea
        label="Instructions"
        name="instructions"
        rows={8}
        defaultValue={project.instructions}
        hint="Every chat in this project starts with these, before the chat's own instructions."
        className="min-h-40"
      />
      <Footer busy={save.isPending} error={save.error} saved={save.saved} />
    </form>
  );
}

function Select({
  label,
  hint,
  children,
  ...rest
}: {
  label: string;
  hint?: ReactNode;
  children: ReactNode;
} & SelectHTMLAttributes<HTMLSelectElement>) {
  const id = useId();
  return (
    <div>
      <label htmlFor={id} className="field-label">
        {label}
      </label>
      <select
        id={id}
        aria-describedby={hint ? `${id}-hint` : undefined}
        className="field-input"
        {...rest}
      >
        {children}
      </select>
      {hint ? (
        <p id={`${id}-hint`} className="field-hint">
          {hint}
        </p>
      ) : null}
    </div>
  );
}

/** The model new chats start with, which also sets the capacity bar's window. */
export function ModelPanel({ project }: { project: ProjectDetailsResponse }) {
  const models = useQuery({ queryKey: keys.models, queryFn: ({ signal }) => fetchModels(signal) });
  const save = useSave(project);
  const [modelId, setModelId] = useState<string | null>(project.defaultModelId);
  const [reasoning, setReasoning] = useState<ReasoningLevel | null>(project.reasoning);
  const list = models.data ?? [];
  const model = list.find((item) => item.id === modelId);
  const levels = model ? reasoningLevels(model) : [];
  const missing = modelId !== null && models.isSuccess && !model;

  return (
    <form
      noValidate
      className="space-y-4"
      onSubmit={(event) => {
        event.preventDefault();
        const level = reasoning !== null && levels.includes(reasoning) ? reasoning : null;
        save.mutate(modelBody(project, modelId, model ? level : reasoning));
      }}
    >
      <Select
        label="Default model"
        value={modelId ?? ""}
        disabled={!models.isSuccess}
        onChange={(event) => setModelId(event.target.value || null)}
        hint={
          missing
            ? "This model is turned off or no longer listed. Choose another one."
            : "New chats in this project start with it. Its context window sets the files' capacity bar."
        }
      >
        <option value="">None (use the usual default)</option>
        {missing ? <option value={modelId ?? ""}>Unavailable model</option> : null}
        {groupModels(list).map((group) => (
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
          value={reasoning !== null && levels.includes(reasoning) ? reasoning : ""}
          onChange={(event) => setReasoning((event.target.value || null) as ReasoningLevel | null)}
          hint="Chats in this project use this level unless they choose their own."
        >
          <option value="">{levelLabel(null, model)}</option>
          {levels.map((level) => (
            <option key={level} value={level}>
              {levelLabel(level)}
            </option>
          ))}
        </Select>
      ) : null}
      <FormAlert>{models.isError ? describeError(models.error).message : null}</FormAlert>
      <Footer busy={save.isPending} error={save.error} saved={save.saved} />
    </form>
  );
}

const BUILTIN: { name: BuiltinTool; label: string; hint: string }[] = [
  {
    name: "schedules",
    label: "Schedules",
    hint: "Lets the assistant schedule prompts, list them, and cancel them. Scheduling and cancelling always ask first.",
  },
];

/** The tool servers and built-in tools every chat of the project uses. */
export function ToolsPanel({ project }: { project: ProjectResponse }) {
  const servers = useQuery({
    queryKey: keys.toolServers,
    queryFn: ({ signal }) => fetchToolServers(signal),
  });
  const save = useSave(project);
  const [chosen, setChosen] = useState<string[]>(project.toolServerIds);
  const [builtin, setBuiltin] = useState<BuiltinTool[]>(project.builtinTools);
  const toggle = <T,>(list: T[], value: T, on: boolean) =>
    on ? [...list.filter((item) => item !== value), value] : list.filter((item) => item !== value);

  return (
    <form
      noValidate
      className="space-y-4"
      onSubmit={(event) => {
        event.preventDefault();
        save.mutate(toolsBody(project, chosen, builtin));
      }}
    >
      <fieldset className="space-y-2">
        <legend className="field-label mb-2">Tool servers</legend>
        {servers.isSuccess && servers.data.length === 0 ? (
          <p className="text-sm text-muted">
            No tool servers yet.{" "}
            <Link to={path.toolServerNew} className="text-link">
              Add tool server
            </Link>
          </p>
        ) : null}
        {(servers.data ?? []).map((server) => (
          <label key={server.id} className="choice-card items-center gap-3">
            <input
              type="checkbox"
              checked={chosen.includes(server.id)}
              onChange={(event) => setChosen(toggle(chosen, server.id, event.target.checked))}
            />
            <span className="min-w-0 flex-1 truncate">{server.name}</span>
            {server.enabled ? null : <Badge>Off</Badge>}
            {server.reviewRequired ? <Badge tone="warn">Review</Badge> : null}
          </label>
        ))}
        <FormAlert>{servers.isError ? describeError(servers.error).message : null}</FormAlert>
      </fieldset>
      <fieldset className="space-y-2">
        <legend className="field-label mb-2">Built-in tools</legend>
        {BUILTIN.map((tool) => (
          <label key={tool.name} className="choice-card items-start gap-3">
            <input
              type="checkbox"
              className="mt-1"
              checked={builtin.includes(tool.name)}
              onChange={(event) => setBuiltin(toggle(builtin, tool.name, event.target.checked))}
            />
            <span>
              <span className="block font-medium">{tool.label}</span>
              <span className="block text-muted">{tool.hint}</span>
            </span>
          </label>
        ))}
      </fieldset>
      <p className="text-sm text-muted">A chat can add or remove servers for itself.</p>
      <Footer busy={save.isPending} error={save.error} saved={save.saved} />
    </form>
  );
}
