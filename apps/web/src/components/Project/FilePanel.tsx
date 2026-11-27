import type { Capacity, FileResponse, ProjectDetailsResponse } from "@copilot/contracts";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { isAbort } from "@web/api/client";
import { describeError } from "@web/api/errors";
import {
  deleteFile,
  fetchFileContent,
  fetchProjectFiles,
  moveFile,
  updateFile,
  uploadFile,
} from "@web/api/routes/files";
import { keys } from "@web/api/routes/keys";
import { fetchProjects } from "@web/api/routes/projects";
import Button from "@web/components/Button/Button";
import { ConfirmDialog } from "@web/components/Dialog/ConfirmDialog";
import { Modal } from "@web/components/Dialog/Modal";
import { saveBlob } from "@web/components/Files/save";
import { FormAlert } from "@web/components/Form/FormAlert";
import { Switch } from "@web/components/Form/Switch";
import { PageLoading } from "@web/components/Suspense/PageLoading";
import { log } from "@web/logging";
import {
  addUpload,
  capacityNote,
  capacityText,
  fileDetails,
  percent,
  type Tally,
  tally,
  type Upload,
  uploadPercent,
  uploads,
} from "@web/routes/project/files";
import { projectLabel } from "@web/routes/project/forms";
import { useId, useReducer, useRef, useState } from "react";
import { LuDownload, LuFolderInput, LuTrash2, LuUpload, LuX } from "react-icons/lu";

const BAR_TONE_MAP = {
  unknown: "bg-muted",
  fits: "bg-accent",
  warn: "bg-warn",
  over: "bg-danger",
} as const;

/** The included files' estimated tokens against the default model's context window. */
export function CapacityBar({
  capacity,
  files,
  hasModel,
}: {
  capacity: Capacity;
  files: FileResponse[];
  hasModel: boolean;
}) {
  const id = useId();
  const share = percent(capacity);
  const note = capacityNote(capacity, files, hasModel);
  return (
    <div className="space-y-1.5">
      <div className="flex items-baseline justify-between gap-3 text-sm">
        <span id={id} className="font-medium">
          Context
        </span>
        <span className="text-muted">{capacityText(capacity)}</span>
      </div>
      <meter
        aria-labelledby={id}
        min={0}
        max={100}
        value={Math.min(share ?? 0, 100)}
        className="sr-only"
      >
        {capacityText(capacity)}
      </meter>
      <div aria-hidden className="h-2 overflow-hidden rounded-full bg-raised">
        <div
          className={`h-full rounded-full ${BAR_TONE_MAP[capacity.level]}`}
          style={{ width: `${Math.min(share ?? 0, 100)}%` }}
        />
      </div>
      {note ? (
        <p
          role={capacity.level === "over" ? "alert" : undefined}
          className={
            capacity.level === "over"
              ? "form-alert"
              : capacity.level === "warn"
                ? "form-warning"
                : "text-sm text-muted"
          }
        >
          {note}
        </p>
      ) : null}
    </div>
  );
}

function UploadRow({ upload, onDismiss }: { upload: Upload; onDismiss: () => void }) {
  const share = uploadPercent(upload);
  return (
    <li className="space-y-1 px-4 py-3">
      <div className="flex items-center gap-3 text-sm">
        <span className="min-w-0 flex-1 truncate">{upload.name}</span>
        {upload.status === "uploading" ? (
          <span className="text-muted">{share}%</span>
        ) : (
          <button
            type="button"
            onClick={onDismiss}
            aria-label={`Dismiss ${upload.name}`}
            className="icon-button-sm"
          >
            <LuX aria-hidden />
          </button>
        )}
      </div>
      {upload.status === "uploading" ? (
        <progress
          max={100}
          value={share}
          aria-label={`Uploading ${upload.name}`}
          className="block h-1.5 w-full"
        />
      ) : null}
      {upload.message ? (
        <p
          role={upload.status === "failed" ? "alert" : "status"}
          className={upload.status === "failed" ? "field-error" : "text-sm text-warn"}
        >
          {upload.message}
        </p>
      ) : null}
    </li>
  );
}

function MoveDialog({
  file,
  projectId,
  onClose,
}: {
  file: FileResponse | null;
  projectId: string;
  onClose: () => void;
}) {
  const client = useQueryClient();
  const projects = useQuery({
    queryKey: keys.projectList({}),
    queryFn: ({ signal }) => fetchProjects({}, signal),
    enabled: file !== null,
  });
  const [target, setTarget] = useState("");
  const move = useMutation({
    mutationFn: (destination: string) => moveFile(file?.id ?? "", destination),
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: keys.projects });
      onClose();
    },
  });
  const others = (projects.data ?? []).filter((project) => project.id !== projectId);
  const id = useId();

  return (
    <Modal open={file !== null} title="Move File" onClose={onClose} busy={move.isPending}>
      <form
        className="space-y-4"
        onSubmit={(event) => {
          event.preventDefault();
          if (target) move.mutate(target);
        }}
      >
        <p className="text-sm text-muted">
          Move <span className="font-medium text-fg">{file?.name}</span> into another project. It
          leaves this one; nothing is copied.
        </p>
        {projects.isPending ? <PageLoading /> : null}
        {projects.isSuccess && others.length === 0 ? (
          <p className="text-sm text-muted">There is no other project to move it to.</p>
        ) : null}
        <fieldset className="space-y-2">
          <legend className="sr-only">Project</legend>
          {others.map((project) => (
            <label key={project.id} className="choice-card items-center gap-3">
              <input
                type="radio"
                name={`${id}-project`}
                value={project.id}
                checked={target === project.id}
                onChange={() => setTarget(project.id)}
              />
              <span className="truncate">{projectLabel(project)}</span>
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
          <Button variant="secondary" onClick={onClose} disabled={move.isPending}>
            Cancel
          </Button>
          <Button type="submit" busy={move.isPending} disabled={!target}>
            Move file
          </Button>
        </div>
      </form>
    </Modal>
  );
}

function FileRow({
  file,
  onMove,
  onDelete,
  onChanged,
}: {
  file: FileResponse;
  onMove: () => void;
  onDelete: () => void;
  onChanged: () => void;
}) {
  const [error, setError] = useState<string>();
  const include = useMutation({
    mutationFn: (included: boolean) => updateFile(file.id, { included }),
    onSuccess: onChanged,
    onError: (failure) => setError(describeError(failure).message),
  });
  const download = useMutation({
    mutationFn: () => fetchFileContent(file.id),
    onSuccess: (blob) => saveBlob(blob, file.name),
    onError: (failure) => setError(describeError(failure).message),
  });
  const included = include.isPending ? (include.variables ?? file.included) : file.included;

  return (
    <li className="space-y-2 px-4 py-3">
      <div className="flex flex-wrap items-center gap-3">
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium">{file.name}</p>
          <p className="text-xs text-muted">{fileDetails(file)}</p>
        </div>
        <Switch
          label={`Include ${file.name} in context`}
          hideLabel
          checked={included}
          disabled={include.isPending}
          onChange={(event) => {
            setError(undefined);
            include.mutate(event.target.checked);
          }}
        />
        <div className="flex shrink-0 items-center">
          <button
            type="button"
            onClick={() => {
              setError(undefined);
              download.mutate();
            }}
            disabled={download.isPending}
            aria-label={`Download ${file.name}`}
            title="Download"
            className="icon-button"
          >
            <LuDownload aria-hidden />
          </button>
          <button
            type="button"
            onClick={onMove}
            aria-label={`Move ${file.name}`}
            title="Move to another project"
            className="icon-button"
          >
            <LuFolderInput aria-hidden />
          </button>
          <button
            type="button"
            onClick={onDelete}
            aria-label={`Delete ${file.name}`}
            title="Delete"
            className="icon-button hover:text-danger"
          >
            <LuTrash2 aria-hidden />
          </button>
        </div>
      </div>
      {error ? (
        <p role="alert" className="field-error">
          {error}
        </p>
      ) : null}
    </li>
  );
}

let uploadCount = 0;

/**
 * A project's files: the capacity bar, uploads with their progress (several at once, by picking
 * or dropping files), and each file with its Include in context switch, download, move, and
 * delete.
 */
export function FilePanel({ project }: { project: ProjectDetailsResponse }) {
  const client = useQueryClient();
  const files = useQuery({
    queryKey: keys.projectFiles(project.id),
    queryFn: ({ signal }) => fetchProjectFiles(project.id, signal),
  });
  const [pending, dispatch] = useReducer(uploads, []);
  const [moving, setMoving] = useState<FileResponse | null>(null);
  const [removing, setRemoving] = useState<FileResponse | null>(null);
  const [dragging, setDragging] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  // The server's capacity plus uploads finished since it was read.
  const counted = useRef<Tally | null>(null);
  counted.current = tally(counted.current, project.capacity);

  const refresh = () => client.invalidateQueries({ queryKey: keys.project(project.id) });

  const remove = useMutation({
    mutationFn: (file: FileResponse) => deleteFile(file.id),
    onSuccess: () => {
      setRemoving(null);
      void refresh();
    },
  });

  async function send(file: File) {
    uploadCount += 1;
    const key = `upload-${uploadCount}`;
    dispatch({ type: "start", key, name: file.name, total: file.size });
    try {
      const saved = await uploadFile({ projectId: project.id }, file, file.name, {
        onProgress: ({ loaded, total }) => dispatch({ type: "progress", key, loaded, total }),
      });
      let warning: string | null = null;
      if (saved.included && counted.current) {
        const added = addUpload(counted.current, saved.tokenEstimate);
        counted.current = added.next;
        warning = added.warning;
      }
      dispatch({ type: "done", key, warning: warning ?? undefined });
    } catch (error) {
      if (isAbort(error)) {
        dispatch({ type: "dismiss", key });
        return;
      }
      log("warn", "web.files.upload.failed", { projectId: project.id });
      dispatch({ type: "failed", key, message: describeError(error).message });
    } finally {
      void refresh();
    }
  }

  function pick(list: FileList | null) {
    for (const file of Array.from(list ?? [])) void send(file);
  }

  const list = files.data ?? [];

  return (
    <div className="space-y-4">
      <CapacityBar
        capacity={project.capacity}
        files={list}
        hasModel={project.defaultModelId !== null}
      />

      <section
        aria-label="Add files"
        onDragOver={(event) => {
          event.preventDefault();
          setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(event) => {
          event.preventDefault();
          setDragging(false);
          pick(event.dataTransfer.files);
        }}
        className={`flex flex-col items-center gap-2 rounded-xl border border-dashed px-4 py-5 text-center text-sm ${
          dragging ? "border-accent bg-accent/10" : "border-line"
        }`}
      >
        <input
          ref={input}
          type="file"
          multiple
          className="sr-only"
          tabIndex={-1}
          aria-hidden
          onChange={(event) => {
            pick(event.target.files);
            event.target.value = "";
          }}
        />
        <Button variant="secondary" onClick={() => input.current?.click()}>
          <LuUpload aria-hidden />
          Upload files
        </Button>
        <p className="text-muted">
          Or drop them here. Text, Markdown, code, PDF, images, and audio.
        </p>
      </section>

      {pending.length > 0 ? (
        <ul aria-label="Uploads" className="list-panel">
          {pending.map((upload) => (
            <UploadRow
              key={upload.key}
              upload={upload}
              onDismiss={() => dispatch({ type: "dismiss", key: upload.key })}
            />
          ))}
        </ul>
      ) : null}

      {files.isPending ? <PageLoading /> : null}
      <FormAlert>{files.isError ? describeError(files.error).message : null}</FormAlert>
      {files.isSuccess && list.length === 0 ? (
        <p className="text-sm text-muted">
          No files yet. Every chat in this project gets the files you include.
        </p>
      ) : null}
      {list.length > 0 ? (
        <ul aria-label="Files" className="list-panel">
          {list.map((file) => (
            <FileRow
              key={file.id}
              file={file}
              onMove={() => setMoving(file)}
              onDelete={() => {
                remove.reset();
                setRemoving(file);
              }}
              onChanged={() => void refresh()}
            />
          ))}
        </ul>
      ) : null}

      <MoveDialog
        key={moving?.id ?? "none"}
        file={moving}
        projectId={project.id}
        onClose={() => setMoving(null)}
      />
      <ConfirmDialog
        open={removing !== null}
        title="Delete File?"
        confirm="Delete file"
        busy={remove.isPending}
        error={remove.error ? describeError(remove.error).message : undefined}
        onConfirm={() => {
          if (removing) remove.mutate(removing);
        }}
        onCancel={() => setRemoving(null)}
      >
        <p>{removing?.name} is deleted from this project with its contents.</p>
      </ConfirmDialog>
    </div>
  );
}
