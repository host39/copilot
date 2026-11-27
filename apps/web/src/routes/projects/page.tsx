import type { ProjectResponse } from "@copilot/contracts";
import { useQuery } from "@tanstack/react-query";
import { keys } from "@web/api/routes/keys";
import { fetchProjects } from "@web/api/routes/projects";
import { Badge } from "@web/components/Badge/Badge";
import { PageTitle } from "@web/components/Layout/PageTitle";
import { QueryState } from "@web/components/Settings/QueryState";
import path, { link } from "@web/routes/path";
import { ago } from "@web/routes/settings/format";
import { LuChevronRight, LuFolder, LuPlus } from "react-icons/lu";
import { Link } from "react-router-dom";

function Row({ project }: { project: ProjectResponse }) {
  return (
    <li>
      <Link to={link.project(project.id)} className="list-row">
        <span aria-hidden className="flex size-9 shrink-0 items-center justify-center text-xl">
          {project.icon || <LuFolder className="size-5 text-muted" />}
        </span>
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-center gap-2 text-sm font-medium">
            <span className="break-words">{project.name}</span>
            {project.pinned ? <Badge>Pinned</Badge> : null}
          </p>
          <p className="truncate text-sm text-muted">
            {project.instructions.trim() || "No instructions"}
          </p>
        </div>
        <span className="hidden shrink-0 text-xs text-muted sm:inline">
          {`Updated ${ago(project.updatedAt)}`}
        </span>
        <LuChevronRight aria-hidden className="shrink-0 text-muted" />
      </Link>
    </li>
  );
}

/** All projects, most recently updated first. */
export default function ProjectsPage() {
  const projects = useQuery({
    queryKey: keys.projectList({}),
    queryFn: ({ signal }) => fetchProjects({}, signal),
  });
  const list = projects.data ?? [];

  return (
    <section className="screen max-w-3xl">
      <PageTitle page="Projects" />
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">Projects</h1>
          <p className="mt-1 text-sm text-muted">
            Instructions, files, a default model, and tools that every chat in a project shares.
          </p>
        </div>
        <Link to={path.projectNew} className="button-link">
          <LuPlus aria-hidden />
          New project
        </Link>
      </div>
      <div className="mt-6">
        <QueryState query={projects}>
          {list.length === 0 ? (
            <div className="card-panel text-center">
              <p className="text-sm font-medium">No projects yet</p>
              <p className="mt-1 text-sm text-muted">
                A project keeps a set of files and instructions for many chats. One without files
                works as a reusable prompt.
              </p>
            </div>
          ) : (
            <ul className="list-panel">
              {list.map((project) => (
                <Row key={project.id} project={project} />
              ))}
            </ul>
          )}
        </QueryState>
      </div>
    </section>
  );
}
