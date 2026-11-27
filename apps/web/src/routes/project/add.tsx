import { useQueryClient } from "@tanstack/react-query";
import { keys } from "@web/api/routes/keys";
import { createProject } from "@web/api/routes/projects";
import Button from "@web/components/Button/Button";
import { FormAlert } from "@web/components/Form/FormAlert";
import { TextArea } from "@web/components/Form/TextArea";
import { TextField } from "@web/components/Form/TextField";
import { PageTitle } from "@web/components/Layout/PageTitle";
import { useSubmit } from "@web/hooks/useSubmit";
import path, { link } from "@web/routes/path";
import { NAME_MAX_CHARS, projectSchema } from "@web/routes/project/forms";
import { LuArrowLeft } from "react-icons/lu";
import { Link, useNavigate } from "react-router-dom";

/** New project: a name, an optional emoji, and instructions. Files, model, and tools come next. */
export function AddProject() {
  const navigate = useNavigate();
  const client = useQueryClient();
  const form = useSubmit(projectSchema, createProject, (project) => {
    void client.invalidateQueries({ queryKey: keys.projects });
    navigate(link.project(project.id), { replace: true });
  });

  return (
    <section className="screen max-w-2xl">
      <PageTitle page="New Project" />
      <Link to={path.projects} className="back-link">
        <LuArrowLeft aria-hidden />
        Projects
      </Link>
      <h1 className="text-xl font-semibold tracking-tight">New Project</h1>
      <p className="mt-1 text-sm text-muted">
        Add files, a default model, and tools on the project's page once it exists.
      </p>
      <form
        noValidate
        className="card-panel mt-6 space-y-4"
        onSubmit={(event) => {
          event.preventDefault();
          const data = new FormData(event.currentTarget);
          void form.submit({
            name: data.get("name") ?? "",
            icon: data.get("icon") ?? "",
            instructions: data.get("instructions") ?? "",
          });
        }}
      >
        <FormAlert>{form.alert}</FormAlert>
        <div className="grid gap-4 sm:grid-cols-[1fr_8rem]">
          <TextField
            label="Name"
            name="name"
            required
            maxLength={NAME_MAX_CHARS}
            autoComplete="off"
            error={form.errors.name}
          />
          <TextField
            label="Icon"
            name="icon"
            placeholder="📁"
            autoComplete="off"
            hint="One emoji"
            error={form.errors.icon}
          />
        </div>
        <TextArea
          label="Instructions"
          name="instructions"
          rows={6}
          hint="The system prompt of every chat in the project."
          error={form.errors.instructions}
        />
        <Button type="submit" busy={form.busy} disabled={form.wait > 0}>
          Create project
        </Button>
      </form>
    </section>
  );
}
