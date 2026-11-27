// Project files and chat attachments (`/projects/{id}/files`, `/chats/{id}/files`, `/files/{id}`).
// Uploads are `multipart/form-data` with one part named `file`; the server checks the type, the
// size, and the storage quota. The client reports an upload's progress where its transport can
// (`RequestOptions.onUploadProgress`).

import {
  type FileResponse,
  fileBodySchema,
  fileListBodySchema,
  UPLOAD_FIELD,
  type UpdateFileRequest,
} from "@copilot/contracts";
import { apiFetch, apiRequest, type UploadProgress } from "@web/api/client";
import API from "@web/api/endpoints";

export type { UploadProgress } from "@web/api/client";

/** `GET /projects/{id}/files`: oldest first. */
export async function fetchProjectFiles(
  projectId: string,
  signal?: AbortSignal
): Promise<FileResponse[]> {
  return (await apiFetch(API.projectFiles(projectId), fileListBodySchema, { signal })).items;
}

/** `PATCH /files/{id}`: the name, or (project files only) whether chats include it. */
export async function updateFile(fileId: string, body: UpdateFileRequest): Promise<FileResponse> {
  return (await apiFetch(API.file(fileId), fileBodySchema, { method: "PATCH", body })).data;
}

/** `POST /files/{id}/move`: into another project, without a copy. */
export async function moveFile(fileId: string, projectId: string): Promise<FileResponse> {
  return (
    await apiFetch(API.fileMove(fileId), fileBodySchema, {
      method: "POST",
      body: { projectId },
    })
  ).data;
}

/** `DELETE /files/{id}`: the file and its bytes. */
export async function deleteFile(fileId: string): Promise<void> {
  await apiRequest(API.file(fileId), { method: "DELETE" });
}

/** `GET /files/{id}/content`: the bytes, sent with the token (never in a URL). */
export async function fetchFileContent(fileId: string, signal?: AbortSignal): Promise<Blob> {
  const response = await apiRequest(API.fileContent(fileId), { signal, accept: "*/*" });
  return response.blob();
}

/** Where an upload goes. */
export type UploadTarget = { projectId: string } | { chatId: string };

export type UploadOptions = {
  onProgress?: (progress: UploadProgress) => void;
  signal?: AbortSignal;
};

function uploadPath(target: UploadTarget): string {
  return "projectId" in target ? API.projectFiles(target.projectId) : API.chatFiles(target.chatId);
}

/**
 * `POST /projects/{id}/files` or `POST /chats/{id}/files`: stores one file and returns it.
 * `onProgress` hears the bytes sent while the browser uploads, and the whole size once done.
 */
export async function uploadFile(
  target: UploadTarget,
  file: Blob,
  name: string,
  options: UploadOptions = {}
): Promise<FileResponse> {
  const body = new FormData();
  body.append(UPLOAD_FIELD, file, name);
  const saved = await apiFetch(uploadPath(target), fileBodySchema, {
    method: "POST",
    body,
    signal: options.signal,
    onUploadProgress: options.onProgress,
  });
  options.onProgress?.({ loaded: file.size, total: file.size });
  return saved.data;
}
