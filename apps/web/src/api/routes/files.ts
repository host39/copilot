// Project files and chat attachments (`/projects/{id}/files`, `/chats/{id}/files`, `/files/{id}`).
// Uploads are `multipart/form-data` with one part named `file`; the server checks the type, the
// size, and the storage quota.
//
// `fetch` cannot report upload progress, so browser uploads go through `XMLHttpRequest`, built the
// way `api/client.ts` builds a request: the configured server, the bearer token, a request id, and
// the same `ApiError` for a failure (a `401 UNAUTHORIZED` for the current token signs out). Native
// builds send through the client's transport instead (plugin-http's `fetch`), without byte
// progress. An interface change in `client.ts` could carry progress and retire this copy; see
// `docs/design/tracks/d.md`.

import {
  type ErrorResponse,
  errorResponseSchema,
  type FileResponse,
  fileBodySchema,
  fileListBodySchema,
  REQUEST_ID_HEADER,
  retryAfterSecs,
  UPLOAD_FIELD,
  type UpdateFileRequest,
} from "@copilot/contracts";
import {
  ApiError,
  apiFetch,
  apiRequest,
  INVALID_RESPONSE,
  NETWORK_ERROR,
  NO_SERVER,
  requestId,
} from "@web/api/client";
import API from "@web/api/endpoints";
import { clearToken, getToken } from "@web/auth/token";
import { API_PREFIX, isNative, serverUrl } from "@web/config";
import { log } from "@web/logging";

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

/** Bytes sent so far, of `total`. */
export type UploadProgress = { loaded: number; total: number };

export type UploadOptions = {
  onProgress?: (progress: UploadProgress) => void;
  signal?: AbortSignal;
};

type RequestFactory = () => XMLHttpRequest;

function browserRequest(): RequestFactory | null {
  if (typeof XMLHttpRequest !== "function" || isNative()) return null;
  return () => new XMLHttpRequest();
}

let factory: RequestFactory | null | undefined;

/**
 * Replaces how uploads build their `XMLHttpRequest` (tests); `null` sends through the client's
 * transport without progress, and `undefined` restores the default.
 */
export function setUploadRequest(next: RequestFactory | null | undefined): void {
  factory = next;
}

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
  const path = uploadPath(target);
  const body = new FormData();
  body.append(UPLOAD_FIELD, file, name);
  const make = factory === undefined ? browserRequest() : factory;
  const saved = make
    ? await send(make(), path, body, options)
    : (await apiFetch(path, fileBodySchema, { method: "POST", body, signal: options.signal })).data;
  options.onProgress?.({ loaded: file.size, total: file.size });
  return saved;
}

function json(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return undefined;
  }
}

function seconds(value: string | null): number | undefined {
  if (value === null || !/^\d+$/.test(value.trim())) return undefined;
  return Number(value.trim());
}

/** The answer of a finished upload: the stored file, or the `ApiError` `apiRequest` would throw. */
export function readUpload(
  status: number,
  text: string,
  header: (name: string) => string | null,
  id: string
): FileResponse | ApiError {
  const body = json(text);
  const requestHeader = header(REQUEST_ID_HEADER) ?? id;
  if (status >= 200 && status < 300) {
    const parsed = fileBodySchema.safeParse(body);
    if (parsed.success) return parsed.data.data;
    return new ApiError(status, {
      error: "The server sent an unexpected response.",
      code: INVALID_RESPONSE,
      requestId: requestHeader,
    });
  }
  const parsed = errorResponseSchema.safeParse(body);
  const error: ErrorResponse = parsed.success
    ? parsed.data
    : { error: `The server answered with status ${status}.` };
  error.requestId ??= requestHeader;
  return new ApiError(status, error, retryAfterSecs(error) ?? seconds(header("Retry-After")));
}

function send(
  request: XMLHttpRequest,
  path: string,
  body: FormData,
  { onProgress, signal }: UploadOptions
): Promise<FileResponse> {
  const server = serverUrl();
  if (server === undefined) {
    return Promise.reject(
      new ApiError(0, { error: "Connect to a Copilot server first.", code: NO_SERVER })
    );
  }
  const id = requestId();
  const token = getToken();

  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(new DOMException("The upload was cancelled.", "AbortError"));
      return;
    }
    request.open("POST", `${server}${API_PREFIX}${path}`);
    request.setRequestHeader("Accept", "application/json");
    request.setRequestHeader(REQUEST_ID_HEADER, id);
    if (token) request.setRequestHeader("Authorization", `Bearer ${token}`);

    const stop = () => request.abort();
    signal?.addEventListener("abort", stop, { once: true });
    const finish = () => signal?.removeEventListener("abort", stop);

    request.upload.onprogress = (event) => {
      if (event.lengthComputable) onProgress?.({ loaded: event.loaded, total: event.total });
    };
    request.onload = () => {
      finish();
      const answer = readUpload(
        request.status,
        request.responseText,
        (name) => request.getResponseHeader(name),
        id
      );
      if (!(answer instanceof ApiError)) {
        resolve(answer);
        return;
      }
      log("warn", "web.api.request.failed", {
        path,
        status: answer.status,
        code: answer.code,
        requestId: answer.requestId,
      });
      const rejected = answer.status === 401 && answer.code === "UNAUTHORIZED";
      if (rejected && token !== null && getToken() === token) clearToken();
      reject(answer);
    };
    request.onerror = () => {
      finish();
      log("warn", "web.api.request.unreachable", { path, requestId: id });
      reject(
        new ApiError(0, {
          error: "The server cannot be reached.",
          code: NETWORK_ERROR,
          requestId: id,
        })
      );
    };
    request.onabort = () => {
      finish();
      reject(new DOMException("The upload was cancelled.", "AbortError"));
    };
    request.send(body);
  });
}
