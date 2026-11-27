// The one HTTP client of the web app. Every request goes through `apiRequest`, which builds the URL
// from the server URL, sends the bearer token, and turns failures into `ApiError`. `apiFetch`
// parses JSON bodies with a contracts schema; `apiStream` reads the chat routes' event streams.
//
// The transport is injectable: web builds use the browser's `fetch`; native builds install the
// `fetch` of `@tauri-apps/plugin-http` at startup (no CORS, mixed content, or Android cleartext
// limits), and tests install a fake. `fetch` cannot report the bytes an upload sent, so a request
// that asks for upload progress goes through `XMLHttpRequest` in browsers (`Transport.upload`);
// native builds have none and send it with their `fetch`, without byte progress.

import {
  type ErrorResponse,
  errorResponseSchema,
  REQUEST_ID_HEADER,
  retryAfterSecs,
} from "@copilot/contracts";
import { readSse } from "@web/api/sse";
import { clearToken, getToken } from "@web/auth/token";
import { API_PREFIX, serverUrl } from "@web/config";
import { log } from "@web/logging";
import type { z } from "zod";

export type Fetch = (input: string, init?: RequestInit) => Promise<Response>;

/** Bytes an upload sent so far, of `total`. */
export type UploadProgress = { loaded: number; total: number };

export type Transport = {
  fetch: Fetch;
  /**
   * Builds the request an upload with progress goes through (`XMLHttpRequest`); `null` sends
   * uploads with `fetch`, without byte progress.
   */
  upload: (() => XMLHttpRequest) | null;
  /** The server URL without a trailing slash; `""` for the page's own origin. */
  server: () => string | undefined;
  token: () => string | null;
  /** Called when the server rejects the token that is still current (`401 UNAUTHORIZED`). */
  signedOut: () => void;
};

function browserUpload(): (() => XMLHttpRequest) | null {
  return typeof XMLHttpRequest === "function" ? () => new XMLHttpRequest() : null;
}

const browserTransport: Transport = {
  fetch: (input, init) => globalThis.fetch(input, init),
  upload: browserUpload(),
  server: serverUrl,
  token: getToken,
  signedOut: clearToken,
};

let transport: Transport = browserTransport;

/** Replaces parts of the transport (native `fetch`, test fakes). */
export function setTransport(next: Partial<Transport>): void {
  transport = { ...transport, ...next };
}

/** Restores the browser transport (tests). */
export function resetTransport(): void {
  transport = browserTransport;
}

/** `ApiError.code` when the request never reached the server. */
export const NETWORK_ERROR = "NETWORK_ERROR";
/** `ApiError.code` when a success response did not match its schema. */
export const INVALID_RESPONSE = "INVALID_RESPONSE";
/** `ApiError.code` for a native build that has no server URL yet. */
export const NO_SERVER = "NO_SERVER";

/** A failed request. `message` is the server's human-readable `error`, safe to show as is. */
export class ApiError extends Error {
  readonly status: number;
  readonly code: string | undefined;
  readonly field: string | undefined;
  readonly requestId: string | undefined;
  /** Seconds to wait before retrying, for `429` answers. */
  readonly retryAfterSecs: number | undefined;

  constructor(status: number, body: ErrorResponse, retryAfterSecs?: number) {
    super(body.error);
    this.name = "ApiError";
    this.status = status;
    this.code = body.code;
    this.field = body.field;
    this.requestId = body.requestId;
    this.retryAfterSecs = retryAfterSecs;
  }
}

/** Whether `error` comes from an aborted request or stream (the user pressed Stop, a page left). */
export function isAbort(error: unknown): boolean {
  return error instanceof Error && error.name === "AbortError";
}

type Query = Record<string, string | number | boolean | null | undefined>;

export type RequestOptions = {
  method?: "GET" | "POST" | "PUT" | "PATCH" | "DELETE";
  /** Sent as JSON, or as is for `FormData` (uploads). */
  body?: unknown;
  /** Query parameters; `null` and `undefined` values are left out. */
  query?: Query;
  signal?: AbortSignal;
  /** Relative to the server root instead of `/api` (`/health`). */
  root?: boolean;
  /** Another server URL to call (the Connect screen's check). It never gets the token. */
  server?: string;
  accept?: string;
  /** Hears the bytes the body sent, where the transport can tell (`Transport.upload`). */
  onUploadProgress?: (progress: UploadProgress) => void;
};

/** A random version 4 UUID, also outside secure contexts (plain `http` on a LAN), where `crypto.randomUUID` is missing. */
export function requestId(): string {
  if (typeof crypto.randomUUID === "function") return crypto.randomUUID();
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = ((bytes[6] ?? 0) & 0x0f) | 0x40;
  bytes[8] = ((bytes[8] ?? 0) & 0x3f) | 0x80;
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function encode(query: Query | undefined): string {
  const params = new URLSearchParams();
  for (const [name, value] of Object.entries(query ?? {})) {
    if (value !== null && value !== undefined) params.append(name, String(value));
  }
  const text = params.toString();
  return text ? `?${text}` : "";
}

function origin(url: string): string {
  return new URL(url || "/", globalThis.location?.href ?? "http://localhost/").origin;
}

function seconds(value: string | null): number | undefined {
  if (value === null || !/^\d+$/.test(value.trim())) return undefined;
  return Number(value.trim());
}

async function readError(response: Response): Promise<ApiError> {
  const json: unknown = await response.json().catch(() => undefined);
  const parsed = errorResponseSchema.safeParse(json);
  const body: ErrorResponse = parsed.success
    ? parsed.data
    : { error: `The server answered with status ${response.status}.` };
  body.requestId ??= response.headers.get(REQUEST_ID_HEADER) ?? undefined;
  const retryAfter = retryAfterSecs(body) ?? seconds(response.headers.get("Retry-After"));
  return new ApiError(response.status, body, retryAfter);
}

type Prepared = { url: string; init: RequestInit; token: string | null; id: string };

/** Whether a response with `status` has no body (`new Response` refuses one). */
function bodiless(status: number): boolean {
  return status === 204 || status === 205 || status === 304;
}

/** The headers of `getAllResponseHeaders()`, one `name: value` per line. */
export function responseHeaders(text: string): Headers {
  const headers = new Headers();
  for (const line of text.split(/\r?\n/)) {
    const colon = line.indexOf(":");
    if (colon > 0) {
      try {
        headers.append(line.slice(0, colon).trim(), line.slice(colon + 1).trim());
      } catch {
        // A header `Headers` refuses carries nothing the client reads.
      }
    }
  }
  return headers;
}

/**
 * `fetch` over `XMLHttpRequest`, which reports the bytes the body sent. Answers a `Response` as
 * `fetch` does, rejects with a `TypeError` when the server cannot be reached and with an
 * `AbortError` once `init.signal` aborts.
 */
export function sendWithProgress(
  request: XMLHttpRequest,
  url: string,
  init: RequestInit,
  onProgress: (progress: UploadProgress) => void
): Promise<Response> {
  return new Promise((resolve, reject) => {
    const signal = init.signal;
    const aborted = () => new DOMException("The request was cancelled.", "AbortError");
    if (signal?.aborted) {
      reject(aborted());
      return;
    }
    request.open(init.method ?? "GET", url);
    new Headers(init.headers).forEach((value, name) => {
      request.setRequestHeader(name, value);
    });
    const stop = () => request.abort();
    signal?.addEventListener("abort", stop, { once: true });
    const finish = () => signal?.removeEventListener("abort", stop);

    request.upload.onprogress = (event) => {
      if (event.lengthComputable) onProgress({ loaded: event.loaded, total: event.total });
    };
    request.onload = () => {
      finish();
      const status = request.status;
      if (status < 200 || status > 599) {
        reject(new TypeError("The request failed."));
        return;
      }
      resolve(
        new Response(bodiless(status) ? null : request.responseText, {
          status,
          statusText: request.statusText,
          headers: responseHeaders(request.getAllResponseHeaders()),
        })
      );
    };
    request.onerror = () => {
      finish();
      reject(new TypeError("The request failed."));
    };
    request.onabort = () => {
      finish();
      reject(aborted());
    };
    request.send((init.body ?? null) as XMLHttpRequestBodyInit | null);
  });
}

/** How a request goes out: with upload progress where it asks for it and the transport can. */
function sender(options: RequestOptions): Fetch {
  const progress = options.onUploadProgress;
  const upload = transport.upload;
  if (!progress || !upload) return transport.fetch;
  return (url, init = {}) => sendWithProgress(upload(), url, init, progress);
}

/** The URL, headers, and body of a request. The token goes only to the configured server. */
function prepare(path: string, options: RequestOptions): Prepared {
  const configured = transport.server();
  const server = options.server ?? configured;
  if (server === undefined) {
    throw new ApiError(0, { error: "Connect to a Copilot server first.", code: NO_SERVER });
  }
  const url = `${server}${options.root ? "" : API_PREFIX}${path}${encode(options.query)}`;
  const id = requestId();
  const headers = new Headers({ Accept: options.accept ?? "application/json" });
  headers.set(REQUEST_ID_HEADER, id);

  const token =
    configured !== undefined && origin(url) === origin(configured) ? transport.token() : null;
  if (token) headers.set("Authorization", `Bearer ${token}`);

  let body: BodyInit | undefined;
  if (options.body instanceof FormData) {
    body = options.body;
  } else if (options.body !== undefined) {
    headers.set("Content-Type", "application/json");
    body = JSON.stringify(options.body);
  }
  const init = { method: options.method ?? "GET", headers, body, signal: options.signal };
  return { url, init, token, id };
}

/**
 * Sends a request and returns the response when its status is 2xx; throws `ApiError` otherwise.
 * A `401 UNAUTHORIZED` for the token that is still current signs the user out (the session
 * expired or was revoked).
 */
export async function apiRequest(path: string, options: RequestOptions = {}): Promise<Response> {
  const { url, init, token, id } = prepare(path, options);

  let response: Response;
  try {
    response = await sender(options)(url, init);
  } catch (error) {
    if (isAbort(error)) throw error;
    log("warn", "web.api.request.unreachable", { path, requestId: id });
    throw new ApiError(0, {
      error: "The server cannot be reached.",
      code: NETWORK_ERROR,
      requestId: id,
    });
  }

  if (response.ok) return response;

  const error = await readError(response);
  log("warn", "web.api.request.failed", {
    path,
    status: error.status,
    code: error.code,
    requestId: error.requestId ?? id,
  });
  const rejected = response.status === 401 && error.code === "UNAUTHORIZED";
  if (rejected && token !== null && transport.token() === token) transport.signedOut();
  throw error;
}

function invalid(response: Response, path: string): ApiError {
  const requestId = response.headers.get(REQUEST_ID_HEADER) ?? undefined;
  log("error", "web.api.response.invalid", { path, status: response.status, requestId });
  return new ApiError(response.status, {
    error: "The server sent an unexpected response.",
    code: INVALID_RESPONSE,
    requestId,
  });
}

/** Sends a request and parses the JSON success body with `schema` (`undefined` for a `204`). */
export async function apiFetch<S extends z.ZodType>(
  path: string,
  schema: S,
  options: RequestOptions = {}
): Promise<z.output<S>> {
  const response = await apiRequest(path, options);
  let json: unknown;
  if (response.status !== 204) {
    try {
      json = await response.json();
    } catch (error) {
      if (isAbort(error)) throw error;
      throw invalid(response, path);
    }
  }
  const parsed = schema.safeParse(json);
  if (!parsed.success) throw invalid(response, path);
  return parsed.data;
}

/**
 * Sends a request whose answer is an event stream (`text/event-stream`) and calls `onEvent` with
 * each event's JSON `data`, parsed with `schema`, until the stream ends. Events the schema does
 * not know (a newer server) are skipped. Returns `false` when the server answered `204` (no run
 * to attach to), `true` after a stream; `options.onOpen` hears when the stream starts. Abort
 * `options.signal` to stop reading.
 */
export async function apiStream<S extends z.ZodType>(
  path: string,
  schema: S,
  onEvent: (event: z.output<S>) => void,
  options: RequestOptions & { onOpen?: () => void } = {}
): Promise<boolean> {
  const { onOpen, ...rest } = options;
  const response = await apiRequest(path, { ...rest, accept: "text/event-stream" });
  if (response.status === 204 || response.body === null) return false;
  onOpen?.();

  await readSse(
    response.body,
    (message) => {
      let json: unknown;
      try {
        json = JSON.parse(message.data);
      } catch {
        throw invalid(response, path);
      }
      const parsed = schema.safeParse(json);
      if (parsed.success) {
        onEvent(parsed.data);
      } else {
        log("warn", "web.api.stream.event-skipped", { path, event: message.event });
      }
    },
    options.signal
  );
  return true;
}
