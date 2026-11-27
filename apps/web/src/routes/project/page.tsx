import { AddProject } from "@web/routes/project/add";
import { ProjectDetails } from "@web/routes/project/details";
import { useParams } from "react-router-dom";

/** `/projects/new` creates a project; `/projects/:projectId` shows one. */
export default function ProjectPage() {
  const { projectId } = useParams();
  return projectId ? <ProjectDetails key={projectId} id={projectId} /> : <AddProject />;
}
