import type { ProjectDetailsResponse } from "@copilot/contracts";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { keys } from "@web/api/routes/keys";
import { deleteProject, fetchProject, updateProject } from "@web/api/routes/projects";
import Button from "@web/components/Button/Button";
import { StartChat } from "@web/components/Chat/StartChat";
import { ChatRow } from "@web/components/Chats/ChatRow";
import { useChatPages } from "@web/components/Chats/pages";
import { ConfirmDialog } from "@web/components/Dialog/ConfirmDialog";
import { Modal } from "@web/components/Dialog/Modal";
import { FormAlert } from "@web/components/Form/FormAlert";
import { TextField } from "@web/components/Form/TextField";
import { PageTitle } from "@web/components/Layout/PageTitle";
import { FilePanel } from "@web/components/Project/FilePanel";
import { QueryState } from "@web/components/Settings/QueryState";
import { useSubmit } from "@web/hooks/useSubmit";
import path from "@web/routes/path";
import { identitySchema, NAME_MAX_CHARS, projectLabel } from "@web/routes/project/forms";
import { InstructionsPanel, ModelPanel, ToolsPanel } from "@web/routes/project/panels";
import { ago } from "@web/routes/settings/format";
import { type ReactNode, useId, useState } from "react";
import { LuArrowLeft, LuPencil, LuPin, LuPinOff, LuTrash2 } from "react-icons/lu";
import { Link, useNavigate } from "react-router-dom";

function Panel({ title, children }: { title: string; children: ReactNode }) {
  const id = useId();
  return (
    <section aria-labelledby={id} className="rounded-2xl border border-line bg-canvas p-4 sm:p-5">
      <h2 id={id} className="mb-3 text-base font-semibold">
        {title}
      </h2>
      {children}
    </section>
  );
}

function RenameForm({ project, onDone }: { project: ProjectDetailsResponse; onDone: () => void }) {
  const client = useQueryClient();
  const form = useSubmit(
    identitySchema(project),
    async (body) => (Object.keys(body).length === 0 ? project : updateProject(project.id, body)),
    () => {
      void client.invalidateQueries({ queryKey: keys.projects });
      void client.invalidateQueries({ queryKey: keys.chats });
      onDone();
    }
  );
  return (
    <form
      noValidate
      className="space-y-4"
      onSubmit={(event) => {
        event.preventDefault();
        const data = new FormData(event.currentTarget);
        void form.submit({ name: data.get("name") ?? "", icon: data.get("icon") ?? "" });
      }}
    >
      <FormAlert>{form.alert}</FormAlert>
      <TextField
        label="Name"
        name="name"
        defaultValue={project.name}
        maxLength={NAME_MAX_CHARS}
        autoComplete="off"
        error={form.errors.name}
      />
      <TextField
        label="Icon"
        name="icon"
        defaultValue={project.icon}
        placeholder="📁"
        autoComplete="off"
        hint="One emoji, or empty"
        error={form.errors.icon}
      />
      <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
        <Button variant="secondary" onClick={onDone} disabled={form.busy}>
          Cancel
        </Button>
        <Button type="submit" busy={form.busy}>
          Save
        </Button>
      </div>
    </form>
  );
}

function Chats({ project }: { project: ProjectDetailsResponse }) {
  const pages = useChatPages({ projectId: project.id });
  return (
    <section aria-labelledby="project-chats" className="space-y-3">
      <h2 id="project-chats" className="text-base font-semibold">
        Chats
      </h2>
      <FormAlert>{pages.isError ? describeError(pages.error).message : null}</FormAlert>
      {pages.isSuccess && pages.chats.length === 0 ? (
        <p className="text-sm text-muted">No chats in this project yet.</p>
      ) : null}
      {pages.chats.length > 0 ? (
        <ul className="list-panel">
          {pages.chats.map((chat) => (
            <li key={chat.id}>
              <ChatRow chat={chat} variant="list" detail={ago(chat.updatedAt)} />
            </li>
          ))}
        </ul>
      ) : null}
      {pages.hasNextPage ? (
        <Button
          variant="secondary"
          busy={pages.isFetchingNextPage}
          onClick={() => void pages.fetchNextPage()}
        >
          Show more
        </Button>
      ) : null}
    </section>
  );
}

/** The heading with Pin, Rename, and Delete. */
function Header({ project }: { project: ProjectDetailsResponse }) {
  const client = useQueryClient();
  const navigate = useNavigate();
  const [renaming, setRenaming] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const pin = useMutation({
    mutationFn: (pinned: boolean) => updateProject(project.id, { pinned }),
    onSuccess: () => void client.invalidateQueries({ queryKey: keys.projects }),
  });
  const remove = useMutation({
    mutationFn: () => deleteProject(project.id),
    onSuccess: () => {
      client.removeQueries({ queryKey: keys.project(project.id) });
      void client.invalidateQueries({ queryKey: keys.projects });
      void client.invalidateQueries({ queryKey: keys.chats });
      navigate(path.projects, { replace: true });
    },
  });

  return (
    <>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <h1 className="min-w-0 break-words text-xl font-semibold tracking-tight">
          {projectLabel(project)}
        </h1>
        <div className="flex shrink-0 items-center">
          <button
            type="button"
            className="icon-button"
            onClick={() => pin.mutate(!project.pinned)}
            disabled={pin.isPending}
            aria-label={project.pinned ? "Unpin project" : "Pin project"}
            title={project.pinned ? "Unpin" : "Pin to the sidebar"}
          >
            {project.pinned ? <LuPinOff aria-hidden /> : <LuPin aria-hidden />}
          </button>
          <button
            type="button"
            className="icon-button"
            onClick={() => setRenaming(true)}
            aria-label="Rename project"
            title="Rename"
          >
            <LuPencil aria-hidden />
          </button>
          <button
            type="button"
            className="icon-button hover:text-danger"
            onClick={() => {
              remove.reset();
              setDeleting(true);
            }}
            aria-label="Delete project"
            title="Delete"
          >
            <LuTrash2 aria-hidden />
          </button>
        </div>
      </div>
      <FormAlert>{pin.error ? describeError(pin.error).message : null}</FormAlert>

      <Modal open={renaming} title="Rename Project" onClose={() => setRenaming(false)}>
        <RenameForm project={project} onDone={() => setRenaming(false)} />
      </Modal>
      <ConfirmDialog
        open={deleting}
        title="Delete Project?"
        confirm="Delete project"
        busy={remove.isPending}
        error={remove.error ? describeError(remove.error).message : undefined}
        onConfirm={() => remove.mutate()}
        onCancel={() => setDeleting(false)}
      >
        <p>
          {project.name} is deleted with its chats, files, and schedules. This cannot be undone.
        </p>
      </ConfirmDialog>
    </>
  );
}

type Tab = "chats" | "project";

const TABS: readonly [Tab, string][] = [
  ["chats", "Chats"],
  ["project", "Instructions and files"],
];

/**
 * A project: its name and icon, a composer that starts a chat in it, its chats, and a side panel
 * (a tab on phones) with instructions, files and the capacity bar, the default model, and tools.
 */
export function ProjectDetails({ id }: { id: string }) {
  const project = useQuery({
    queryKey: keys.project(id),
    queryFn: ({ signal }) => fetchProject(id, signal),
  });
  const [tab, setTab] = useState<Tab>("chats");
  const data = project.data;
  const shown = (value: Tab) => (tab === value ? "block" : "hidden");

  return (
    <section className="screen max-w-6xl">
      <PageTitle page={data ? data.name : "Project"} />
      <Link
        to={path.projects}
        className="mb-3 inline-flex min-h-10 items-center gap-1.5 rounded-lg text-sm text-muted hover:text-fg"
      >
        <LuArrowLeft aria-hidden />
        Projects
      </Link>
      <QueryState query={project} back={{ to: path.projects, label: "All projects" }}>
        {data ? (
          <>
            <Header project={data} />
            <div role="tablist" aria-label="Project" className="mt-4 flex gap-1 lg:hidden">
              {TABS.map(([value, label]) => (
                <button
                  key={value}
                  type="button"
                  role="tab"
                  id={`project-tab-${value}`}
                  aria-selected={tab === value}
                  aria-controls={`project-panel-${value}`}
                  onClick={() => setTab(value)}
                  className="nav-item aria-selected:bg-raised aria-selected:font-medium"
                >
                  {label}
                </button>
              ))}
            </div>

            <div className="mt-4 grid gap-6 lg:grid-cols-[minmax(0,1fr)_24rem]">
              <div
                id="project-panel-chats"
                role="tabpanel"
                aria-labelledby="project-tab-chats"
                className={`${shown("chats")} min-w-0 space-y-6 lg:block`}
              >
                <StartChat project={data} placeholder={`Start a chat in ${data.name}`} />
                <Chats project={data} />
              </div>
              <aside
                id="project-panel-project"
                role="tabpanel"
                aria-labelledby="project-tab-project"
                className={`${shown("project")} min-w-0 space-y-4 lg:block`}
              >
                <Panel title="Instructions">
                  <InstructionsPanel project={data} />
                </Panel>
                <Panel title="Files">
                  <FilePanel project={data} />
                </Panel>
                <Panel title="Default Model">
                  <ModelPanel project={data} />
                </Panel>
                <Panel title="Tools">
                  <ToolsPanel project={data} />
                </Panel>
              </aside>
            </div>
          </>
        ) : null}
      </QueryState>
    </section>
  );
}
