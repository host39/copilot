// The chat screen's sheets: the chat instructions drawer and Move to project.

import type { ChatResponse, ProjectResponse } from "@copilot/contracts";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { saveInstructions, updateChat } from "@web/api/routes/chats";
import { keys } from "@web/api/routes/keys";
import { fetchProjects } from "@web/api/routes/projects";
import Button from "@web/components/Button/Button";
import { FormAlert } from "@web/components/Form/FormAlert";
import { FormNotice } from "@web/components/Form/FormNotice";
import { TextArea } from "@web/components/Form/TextArea";
import { PageLoading } from "@web/components/Suspense/PageLoading";
import { insertInstructions } from "@web/routes/chat/settings";
import { projectLabel } from "@web/routes/project/forms";
import { useId, useState } from "react";

/** Keeps a saved chat in the cache and refreshes the lists that show it. */
export function useChatSaved() {
  const client = useQueryClient();
  return (saved: ChatResponse, projectsChanged = false) => {
    client.setQueryData(keys.chat(saved.id), saved);
    void client.invalidateQueries({ queryKey: [...keys.chats, "list"] });
    if (projectsChanged) void client.invalidateQueries({ queryKey: keys.projects });
  };
}

function useProjects() {
  return useQuery({
    queryKey: keys.projectList({}),
    queryFn: ({ signal }) => fetchProjects({}, signal),
  });
}

/**
 * The chat instructions drawer: the text sent after the project's instructions (a standalone
 * chat's system prompt), Insert from project (a copy, not a link), and in a project chat, Save to
 * project, which appends them to the project's and clears them here.
 */
export function InstructionsForm({
  chat,
  project,
  onDone,
}: {
  chat: ChatResponse;
  project: ProjectResponse | null;
  onDone: () => void;
}) {
  const saved = useChatSaved();
  const client = useQueryClient();
  const projects = useProjects();
  const [text, setText] = useState(chat.instructions);
  const [notice, setNotice] = useState<string>();
  const id = useId();
  const changed = text.trim() !== chat.instructions;

  const save = useMutation({
    mutationFn: () => updateChat(chat.id, { instructions: text.trim() }),
    onSuccess: (result) => {
      saved(result);
      onDone();
    },
  });
  const toProject = useMutation({
    mutationFn: async () => {
      if (changed) saved(await updateChat(chat.id, { instructions: text.trim() }));
      return saveInstructions(chat.id);
    },
    onSuccess: (updated) => {
      // The chat's instructions are now empty: read it again, without its messages.
      void client.invalidateQueries({ queryKey: keys.chat(chat.id), exact: true });
      void client.invalidateQueries({ queryKey: keys.projects });
      setText("");
      setNotice(`Added to ${updated.name}'s instructions.`);
    },
  });
  const sources = (projects.data ?? []).filter(
    (item) => item.id !== chat.projectId && item.instructions.trim() !== ""
  );
  const error = save.error ?? toProject.error;

  return (
    <form
      noValidate
      className="space-y-4"
      onSubmit={(event) => {
        event.preventDefault();
        if (changed) save.mutate();
        else onDone();
      }}
    >
      {project?.instructions ? (
        <details className="rounded-lg border border-line bg-surface px-3 py-2 text-sm">
          <summary className="cursor-pointer font-medium">{`From ${project.name}`}</summary>
          <p className="mt-2 whitespace-pre-wrap text-muted">{project.instructions}</p>
        </details>
      ) : null}
      <TextArea
        label="Instructions"
        name="instructions"
        rows={10}
        value={text}
        onChange={(event) => setText(event.target.value)}
        hint={
          project
            ? "Sent after the project's instructions, from the next message on."
            : "This chat's system prompt, from the next message on."
        }
        className="min-h-48"
      />
      {sources.length > 0 ? (
        <div>
          <label htmlFor={id} className="field-label">
            Insert from project...
          </label>
          <select
            id={id}
            className="field-input"
            value=""
            onChange={(event) => {
              const source = sources.find((item) => item.id === event.target.value);
              if (source) setText(insertInstructions(text, source.instructions));
            }}
          >
            <option value="">Choose a project</option>
            {sources.map((item) => (
              <option key={item.id} value={item.id}>
                {projectLabel(item)}
              </option>
            ))}
          </select>
          <p className="field-hint">
            Copies its instructions here; later changes to that project do not follow.
          </p>
        </div>
      ) : null}
      <FormAlert>{error ? describeError(error).message : null}</FormAlert>
      <FormNotice>{notice}</FormNotice>
      <div className="flex flex-wrap gap-2">
        <Button type="submit" busy={save.isPending} disabled={toProject.isPending}>
          Save
        </Button>
        {project ? (
          <Button
            variant="secondary"
            busy={toProject.isPending}
            disabled={text.trim() === "" || save.isPending}
            onClick={() => toProject.mutate()}
          >
            Save to project
          </Button>
        ) : null}
      </div>
    </form>
  );
}

/** Move to project: another project, or none. The chat's attachments stay with it. */
export function MoveForm({ chat, onDone }: { chat: ChatResponse; onDone: () => void }) {
  const saved = useChatSaved();
  const projects = useProjects();
  const [target, setTarget] = useState<string>(chat.projectId ?? "");
  const id = useId();
  const move = useMutation({
    mutationFn: () => updateChat(chat.id, { projectId: target || null }),
    onSuccess: (result) => {
      saved(result, true);
      onDone();
    },
  });
  const choices: { id: string; label: string }[] = [
    { id: "", label: "No project" },
    ...(projects.data ?? []).map((item) => ({ id: item.id, label: projectLabel(item) })),
  ];

  return (
    <form
      className="space-y-4"
      onSubmit={(event) => {
        event.preventDefault();
        if (target === (chat.projectId ?? "")) onDone();
        else move.mutate();
      }}
    >
      <p className="text-sm text-muted">
        From the next message on, the chat uses the project's instructions, files, and tools.
      </p>
      {projects.isPending ? <PageLoading /> : null}
      <fieldset className="space-y-2">
        <legend className="sr-only">Project</legend>
        {choices.map((choice) => (
          <label key={choice.id || "none"} className="choice-card items-center gap-3">
            <input
              type="radio"
              name={`${id}-project`}
              value={choice.id}
              checked={target === choice.id}
              onChange={() => setTarget(choice.id)}
            />
            <span className="truncate">{choice.label}</span>
          </label>
        ))}
      </fieldset>
      <FormAlert>
        {move.error
          ? describeError(move.error).message
          : projects.error
            ? describeError(projects.error).message
            : null}
      </FormAlert>
      <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
        <Button variant="secondary" onClick={onDone} disabled={move.isPending}>
          Cancel
        </Button>
        <Button type="submit" busy={move.isPending}>
          Move chat
        </Button>
      </div>
    </form>
  );
}
