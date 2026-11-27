import { useQuery } from "@tanstack/react-query";
import { keys } from "@web/api/routes/keys";
import { fetchProjects } from "@web/api/routes/projects";
import { StartChat } from "@web/components/Chat/StartChat";
import { PageTitle } from "@web/components/Layout/PageTitle";
import { projectLabel } from "@web/routes/project/forms";
import type { StartState } from "@web/routes/welcome/setup";
import { useId } from "react";
import { useLocation, useSearchParams } from "react-router-dom";

/** The search parameter that puts the new chat into a project (the sidebar's `+` per project). */
export const PROJECT_PARAM = "project";

/** The new chat screen for `projectId` (`/?project=<id>`). */
export function newChatLink(projectId: string): string {
  return `/?${new URLSearchParams({ [PROJECT_PARAM]: projectId })}`;
}

/**
 * New chat: the default screen. The draft holds the project, the model, and the reasoning level;
 * the first message creates the chat.
 */
export default function HomePage() {
  const [params, setParams] = useSearchParams();
  const projectId = params.get(PROJECT_PARAM) ?? "";
  const projects = useQuery({
    queryKey: keys.projectList({}),
    queryFn: ({ signal }) => fetchProjects({}, signal),
  });
  // The first-run setup opens this screen with the model it turned on.
  const setup = useLocation().state as StartState | null;
  const project = projects.data?.find((item) => item.id === projectId) ?? null;
  const id = useId();

  const choose = (
    <div className="flex items-center gap-2">
      <label htmlFor={id} className="sr-only">
        Project
      </label>
      <select
        id={id}
        value={project?.id ?? ""}
        disabled={!projects.isSuccess}
        onChange={(event) =>
          setParams(event.target.value ? { [PROJECT_PARAM]: event.target.value } : {}, {
            replace: true,
          })
        }
        className="min-h-10 max-w-56 truncate rounded-lg border border-line bg-canvas px-2 text-sm"
      >
        <option value="">No project</option>
        {(projects.data ?? []).map((item) => (
          <option key={item.id} value={item.id}>
            {projectLabel(item)}
          </option>
        ))}
      </select>
    </div>
  );

  return (
    <section className="screen flex h-full max-w-3xl flex-col gap-6">
      <PageTitle page="New Chat" />
      <h1 className="text-xl font-semibold tracking-tight">
        {project ? `New Chat in ${project.name}` : "New Chat"}
      </h1>
      <div className="mt-auto">
        <StartChat
          key={project?.id ?? "standalone"}
          project={project}
          preferred={setup?.modelId}
          placeholder={project ? `Start a chat in ${project.name}` : undefined}
          extra={choose}
        />
      </div>
    </section>
  );
}
