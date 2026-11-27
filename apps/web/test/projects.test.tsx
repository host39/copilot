// Projects and their files: the request helpers (uploads with progress included), the capacity
// bar's rules, the project forms, the model picker's rules, and the screens.

import type {
  Capacity,
  FileResponse,
  ModelResponse,
  ProjectDetailsResponse,
  ProjectResponse,
  UserResponse,
} from "@copilot/contracts";
import { afterEach, describe, expect, test } from "@rstest/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ApiError, resetTransport, responseHeaders, setTransport } from "@web/api/client";
import {
  deleteFile,
  fetchFileContent,
  fetchProjectFiles,
  moveFile,
  type UploadProgress,
  updateFile,
  uploadFile,
} from "@web/api/routes/files";
import { keys } from "@web/api/routes/keys";
import {
  createProject,
  deleteProject,
  fetchProject,
  fetchProjects,
  updateProject,
} from "@web/api/routes/projects";
import { clearToken, getToken, setToken } from "@web/auth/token";
import {
  fitLevel,
  groupModels,
  levelLabel,
  modelSummary,
  reasoningLevels,
  startModel,
} from "@web/components/ModelPicker/models";
import { fieldErrors } from "@web/forms";
import path from "@web/routes/path";
import {
  addUpload,
  capacityAfter,
  capacityLevel,
  capacityNote,
  capacityText,
  fileDetails,
  largest,
  size,
  tally,
  type Upload,
  uploadPercent,
  uploads,
  uploadWarning,
} from "@web/routes/project/files";
import {
  identitySchema,
  instructionsBody,
  isIcon,
  modelBody,
  projectLabel,
  projectSchema,
  toolsBody,
} from "@web/routes/project/forms";
import ProjectPage from "@web/routes/project/page";
import ProjectsPage from "@web/routes/projects/page";
import { renderToString } from "react-dom/server";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import type { z } from "zod";

const AT = "2026-11-26T10:00:00Z";
const PROJECT = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708200";
const OTHER = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708201";
const FILE = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708210";
const MODEL = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708101";
const SERVER = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708300";

type Call = { url: string; method: string; headers: Headers; body: BodyInit | null | undefined };
let calls: Call[] = [];

function serve(answer: () => Response): void {
  setTransport({
    server: () => "",
    token: () => "token",
    fetch: async (input, init) => {
      calls.push({
        url: input,
        method: init?.method ?? "GET",
        headers: new Headers(init?.headers),
        body: init?.body,
      });
      return answer();
    },
  });
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

/** The one request the last call sent. */
function sent(): { url: string; method: string; body: unknown } {
  expect(calls).toHaveLength(1);
  const call = calls[0] as Call;
  return {
    url: call.url,
    method: call.method,
    body: typeof call.body === "string" ? JSON.parse(call.body) : call.body,
  };
}

const project: ProjectResponse = {
  id: PROJECT,
  name: "Research",
  icon: "📚",
  instructions: "Answer briefly.",
  defaultModelId: null,
  reasoning: null,
  builtinTools: [],
  toolServerIds: [],
  pinned: false,
  createdAt: AT,
  updatedAt: AT,
};

function file(name: string, tokens: number, extra: Partial<FileResponse> = {}): FileResponse {
  return {
    id: FILE,
    projectId: PROJECT,
    chatId: null,
    name,
    mediaType: "text/plain",
    sizeBytes: tokens * 4,
    tokenEstimate: tokens,
    pageCount: null,
    included: true,
    createdAt: AT,
    ...extra,
  };
}

const capabilities = {
  vision: false,
  pdf: false,
  tools: true,
  audioIn: false,
  audioOut: false,
  reasoning: false,
  reasoningLevels: [],
  defaultReasoning: null,
};

function model(id: string, displayName: string, extra: Partial<ModelResponse> = {}): ModelResponse {
  return {
    id,
    providerId: "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708100",
    providerName: "Mock",
    kind: "openai",
    preset: "custom",
    remoteId: displayName.toLowerCase(),
    displayName,
    enabled: true,
    available: true,
    contextWindow: null,
    maxOutputTokens: null,
    capabilities,
    detected: capabilities,
    overrides: {
      vision: null,
      pdf: null,
      tools: null,
      audioIn: null,
      audioOut: null,
      reasoning: null,
    },
    locks: [],
    reasoningOffWithTools: false,
    createdAt: AT,
    updatedAt: AT,
    ...extra,
  };
}

/** The field errors of a failed parse, or the parsed value. */
function parse<S extends z.ZodType>(schema: S, input: unknown) {
  const parsed = schema.safeParse(input);
  return parsed.success ? { value: parsed.data } : { errors: fieldErrors(parsed.error) };
}

afterEach(() => {
  calls = [];
  clearToken();
  resetTransport();
});

describe("the project request helpers", () => {
  test("lists projects with the sidebar's filters", async () => {
    serve(() => json({ items: [project], total: 1 }));
    expect(await fetchProjects({ pinned: false, limit: 5 })).toEqual([project]);
    expect(sent()).toEqual({
      url: "/api/projects?pinned=false&limit=5",
      method: "GET",
      body: undefined,
    });
  });

  test("reads a project with its capacity", async () => {
    const details = { ...project, capacity: { used: 10, window: null, level: "unknown" } };
    serve(() => json({ data: details }));
    expect(await fetchProject(PROJECT)).toEqual(details);
    expect(sent().url).toBe(`/api/projects/${PROJECT}`);
  });

  test("creates, changes, and deletes", async () => {
    serve(() => json({ data: project }, 201));
    await createProject({ name: "Research", icon: "📚" });
    expect(sent()).toEqual({
      url: "/api/projects",
      method: "POST",
      body: { name: "Research", icon: "📚" },
    });

    calls = [];
    serve(() => json({ data: project }));
    await updateProject(PROJECT, { defaultModelId: null, pinned: true });
    expect(sent()).toEqual({
      url: `/api/projects/${PROJECT}`,
      method: "PATCH",
      body: { defaultModelId: null, pinned: true },
    });

    calls = [];
    serve(() => new Response(null, { status: 204 }));
    await deleteProject(PROJECT);
    expect(sent()).toEqual({ url: `/api/projects/${PROJECT}`, method: "DELETE", body: undefined });
  });
});

describe("the file request helpers", () => {
  test("lists, includes, moves, and deletes files", async () => {
    serve(() => json({ items: [file("a.txt", 10)], total: 1 }));
    expect((await fetchProjectFiles(PROJECT))[0]?.name).toBe("a.txt");
    expect(sent().url).toBe(`/api/projects/${PROJECT}/files`);

    calls = [];
    serve(() => json({ data: file("a.txt", 10, { included: false }) }));
    await updateFile(FILE, { included: false });
    expect(sent()).toEqual({
      url: `/api/files/${FILE}`,
      method: "PATCH",
      body: { included: false },
    });

    calls = [];
    serve(() => json({ data: file("a.txt", 10, { projectId: OTHER }) }));
    expect((await moveFile(FILE, OTHER)).projectId).toBe(OTHER);
    expect(sent()).toEqual({
      url: `/api/files/${FILE}/move`,
      method: "POST",
      body: { projectId: OTHER },
    });

    calls = [];
    serve(() => new Response(null, { status: 204 }));
    await deleteFile(FILE);
    expect(sent()).toEqual({ url: `/api/files/${FILE}`, method: "DELETE", body: undefined });
  });

  test("downloads bytes with the token, never through a URL that carries it", async () => {
    serve(() => new Response("hello", { headers: { "Content-Type": "text/plain" } }));
    const blob = await fetchFileContent(FILE);
    expect(await blob.text()).toBe("hello");
    expect(calls[0]?.url).toBe(`/api/files/${FILE}/content`);
    expect(calls[0]?.headers.get("Authorization")).toBe("Bearer token");
  });

  test("without XMLHttpRequest an upload goes through the client as one `file` part", async () => {
    setTransport({ upload: null });
    serve(() => json({ data: file("notes.md", 3) }, 201));
    const seen: UploadProgress[] = [];
    const saved = await uploadFile({ projectId: PROJECT }, new Blob(["# Notes"]), "notes.md", {
      onProgress: (progress) => seen.push(progress),
    });
    expect(saved.name).toBe("notes.md");
    const call = calls[0] as Call;
    expect(call.url).toBe(`/api/projects/${PROJECT}/files`);
    expect(call.method).toBe("POST");
    const body = call.body as FormData;
    expect([...body.keys()]).toEqual(["file"]);
    expect((body.get("file") as File).name).toBe("notes.md");
    expect(seen).toEqual([{ loaded: 7, total: 7 }]);
  });

  test("chat attachments go to the chat's route", async () => {
    setTransport({ upload: null });
    serve(() => json({ data: file("a.png", 1600, { projectId: null, chatId: OTHER }) }, 201));
    await uploadFile({ chatId: OTHER }, new Blob(["x"]), "a.png");
    expect(calls[0]?.url).toBe(`/api/chats/${OTHER}/files`);
  });
});

type ProgressHandler = (event: {
  lengthComputable: boolean;
  loaded: number;
  total: number;
}) => void;

/** Stands in for `XMLHttpRequest`: records the request and answers when told to. */
class FakeRequest {
  static last: FakeRequest | undefined;
  method = "";
  url = "";
  /** Header names in lower case, as `Headers` hands them over. */
  headers: Record<string, string> = {};
  body: unknown;
  status = 0;
  statusText = "";
  responseText = "";
  answerHeaders = "";
  upload: { onprogress: ProgressHandler | null } = { onprogress: null };
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onabort: (() => void) | null = null;

  open(method: string, url: string) {
    this.method = method;
    this.url = url;
  }
  setRequestHeader(name: string, value: string) {
    this.headers[name.toLowerCase()] = value;
  }
  getAllResponseHeaders() {
    return this.answerHeaders;
  }
  send(body: unknown) {
    this.body = body;
    FakeRequest.last = this;
  }
  abort() {
    this.onabort?.();
  }
  respond(status: number, body: unknown, headers: Record<string, string> = {}) {
    this.status = status;
    this.responseText = typeof body === "string" ? body : JSON.stringify(body);
    this.answerHeaders = Object.entries(headers)
      .map(([name, value]) => `${name}: ${value}\r\n`)
      .join("");
    this.onload?.();
  }
}

/** A progress listener, so uploads take the `XMLHttpRequest` path. */
const onProgress = () => undefined;

function fakeRequests(): void {
  FakeRequest.last = undefined;
  setTransport({ upload: () => new FakeRequest() as unknown as XMLHttpRequest });
}

function current(): FakeRequest {
  const request = FakeRequest.last;
  if (!request) throw new Error("no upload was sent");
  return request;
}

describe("uploads with progress", () => {
  test("send the token, a request id, and report each progress event", async () => {
    fakeRequests();
    setToken("token");
    const seen: UploadProgress[] = [];
    const pending = uploadFile({ projectId: PROJECT }, new Blob(["0123456789"]), "a.txt", {
      onProgress: (progress) => seen.push(progress),
    });
    const request = current();
    expect(request.method).toBe("POST");
    expect(request.url).toBe(`/api/projects/${PROJECT}/files`);
    expect(request.headers.authorization).toBe("Bearer token");
    expect(request.headers["x-request-id"]).toMatch(/^[0-9a-f-]{36}$/);
    expect(request.headers.accept).toBe("application/json");
    expect(request.headers["content-type"]).toBeUndefined();
    expect((request.body as FormData).get("file")).toBeInstanceOf(Blob);

    request.upload.onprogress?.({ lengthComputable: true, loaded: 4, total: 10 });
    request.upload.onprogress?.({ lengthComputable: false, loaded: 0, total: 0 });
    request.respond(201, { data: file("a.txt", 3) });
    expect((await pending).name).toBe("a.txt");
    expect(seen).toEqual([
      { loaded: 4, total: 10 },
      { loaded: 10, total: 10 },
    ]);
  });

  test("a request without progress never takes the upload path", async () => {
    fakeRequests();
    serve(() => json({ data: file("a.txt", 3) }));
    await updateFile(FILE, { name: "a.txt" });
    expect(FakeRequest.last).toBeUndefined();
    expect(calls).toHaveLength(1);
  });

  test("a rejected upload throws the server's message and field", async () => {
    fakeRequests();
    const pending = uploadFile({ projectId: PROJECT }, new Blob(["x"]), "a.exe", { onProgress });
    current().respond(
      400,
      { error: "This file type is not supported.", code: "VALIDATION_ERROR", field: "file" },
      { "X-Request-Id": "req-1" }
    );
    const error = await pending.catch((failure: unknown) => failure);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).toMatchObject({
      status: 400,
      message: "This file type is not supported.",
      field: "file",
      requestId: "req-1",
    });
  });

  test("a rejected token signs out, as other requests do", async () => {
    fakeRequests();
    setToken("token");
    const pending = uploadFile({ projectId: PROJECT }, new Blob(["x"]), "a.txt", { onProgress });
    current().respond(401, { error: "Sign in again.", code: "UNAUTHORIZED" });
    await pending.catch(() => undefined);
    expect(getToken()).toBeNull();
  });

  test("network failures and aborts read like the client's", async () => {
    fakeRequests();
    const failing = uploadFile({ projectId: PROJECT }, new Blob(["x"]), "a.txt", { onProgress });
    current().onerror?.();
    expect(await failing.catch((failure: unknown) => failure)).toMatchObject({
      status: 0,
      code: "NETWORK_ERROR",
    });

    const controller = new AbortController();
    const aborted = uploadFile({ projectId: PROJECT }, new Blob(["x"]), "a.txt", {
      signal: controller.signal,
      onProgress,
    });
    controller.abort();
    expect(await aborted.catch((failure: unknown) => (failure as Error).name)).toBe("AbortError");

    const before = new AbortController();
    before.abort();
    const early = uploadFile({ projectId: PROJECT }, new Blob(["x"]), "a.txt", {
      signal: before.signal,
      onProgress,
    });
    expect(await early.catch((failure: unknown) => (failure as Error).name)).toBe("AbortError");
  });

  test("answers the schema does not know, and plain-text failures, read like the client's", async () => {
    fakeRequests();
    const odd = uploadFile({ projectId: PROJECT }, new Blob(["x"]), "a.txt", { onProgress });
    current().respond(201, {}, { "X-Request-Id": "id-1" });
    expect(await odd.catch((failure: unknown) => failure)).toMatchObject({
      code: "INVALID_RESPONSE",
      requestId: "id-1",
    });

    const plain = uploadFile({ projectId: PROJECT }, new Blob(["x"]), "a.txt", { onProgress });
    current().respond(502, "Bad gateway", { "Retry-After": "5" });
    expect(await plain.catch((failure: unknown) => failure)).toMatchObject({
      message: "The server answered with status 502.",
      retryAfterSecs: 5,
    });
  });

  test("response headers parse line by line, skipping what `Headers` refuses", () => {
    const headers = responseHeaders(
      "Content-Type: application/json\r\nX-Request-Id: a:b\r\nbad line\r\n"
    );
    expect(headers.get("content-type")).toBe("application/json");
    expect(headers.get("x-request-id")).toBe("a:b");
  });
});

describe("the capacity bar", () => {
  const at = (used: number, window: number | null = 1_000): Capacity => ({
    used,
    window,
    level: capacityLevel(used, window),
  });

  test("warns past 80% and blocks past 100%, like core", () => {
    expect(capacityLevel(800, 1_000)).toBe("fits");
    expect(capacityLevel(801, 1_000)).toBe("warn");
    expect(capacityLevel(1_000, 1_000)).toBe("warn");
    expect(capacityLevel(1_001, 1_000)).toBe("over");
    expect(capacityLevel(5, null)).toBe("unknown");
    expect(capacityLevel(5, 0)).toBe("unknown");
  });

  test("adds and removes included tokens", () => {
    expect(capacityAfter(at(700), 200)).toEqual({ used: 900, window: 1_000, level: "warn" });
    expect(capacityAfter(at(900), -950)).toEqual({ used: 0, window: 1_000, level: "fits" });
  });

  test("labels the bar with tokens and the share", () => {
    expect(capacityText(at(12_000, 128_000))).toBe("12K of 128K tokens (9%)");
    expect(capacityText(at(12_000, null))).toBe("About 12K tokens");
  });

  test("names the largest included files when sending is blocked", () => {
    const files = [
      file("small.md", 100),
      file("big.pdf", 90_000),
      file("skipped.pdf", 500_000, { included: false }),
      file("mid.txt", 20_000),
      file("tiny.txt", 10),
    ];
    expect(largest(files).map((item) => item.name)).toEqual(["big.pdf", "mid.txt", "small.md"]);
    expect(capacityNote(at(110_110, 100_000), files, true)).toBe(
      "Sending is blocked: the included files do not fit the default model's context window. Turn off or remove some, starting with the largest: big.pdf (90K), mid.txt (20K), small.md (100)."
    );
  });

  test("explains a missing window and the warning", () => {
    expect(capacityNote(at(5, null), [], false)).toContain("Choose a default model");
    expect(capacityNote(at(5, null), [], true)).toContain("context window is unknown");
    expect(capacityNote(at(900), [], true)).toContain("more than 80%");
    expect(capacityNote(at(100), [], true)).toBeNull();
  });

  test("uploads that run at once warn against the total before each", () => {
    const server = at(500);
    let counted = tally(null, server);
    const first = addUpload(counted, 200);
    expect(first.warning).toBeNull();
    counted = tally(first.next, server);
    expect(counted.used).toBe(700);
    const second = addUpload(counted, 200);
    expect(second.warning).toBe(
      "The included files now fill 90% of the default model's context window."
    );
    // A new answer from the server starts the count over from its total.
    expect(tally(second.next, at(900)).used).toBe(900);
  });

  test("an upload warns when it crosses 80% or 100%, once", () => {
    expect(uploadWarning(at(700), 150)).toBe(
      "The included files now fill 85% of the default model's context window."
    );
    expect(uploadWarning(at(900), 200)).toContain("sending is blocked");
    expect(uploadWarning(at(850), 10)).toBeNull();
    expect(uploadWarning(at(10), 10)).toBeNull();
    expect(uploadWarning(at(10, null), 10_000)).toBeNull();
  });
});

describe("file details and uploads", () => {
  test("sizes read in bytes, KB, and MB", () => {
    expect(size(1)).toBe("1 byte");
    expect(size(512)).toBe("512 bytes");
    expect(size(1_536)).toBe("1.5 KB");
    expect(size(25 * 1024 * 1024)).toBe("25 MB");
  });

  test("describes a file by kind, pages, size, and tokens", () => {
    expect(
      fileDetails(
        file("paper.pdf", 36_000, {
          mediaType: "application/pdf",
          pageCount: 12,
          sizeBytes: 1_258_291,
        })
      )
    ).toBe("PDF · 12 pages · 1.2 MB · about 36K tokens");
    expect(fileDetails(file("a.png", 1_600, { mediaType: "image/png", sizeBytes: 2_048 }))).toBe(
      "Image · 2 KB · about 2K tokens"
    );
  });

  test("tracks each upload from start to done or failure", () => {
    let list: Upload[] = [];
    list = uploads(list, { type: "start", key: "a", name: "a.txt", total: 10 });
    list = uploads(list, { type: "start", key: "b", name: "b.txt", total: 0 });
    list = uploads(list, { type: "progress", key: "a", loaded: 20, total: 10 });
    expect(list[0]).toMatchObject({ loaded: 10, status: "uploading" });
    expect(uploadPercent(list[0] as Upload)).toBe(100);
    expect(uploadPercent(list[1] as Upload)).toBe(0);

    // A clean upload leaves the list; one with a warning stays until dismissed.
    expect(uploads(list, { type: "done", key: "a" }).map((item) => item.key)).toEqual(["b"]);
    list = uploads(list, { type: "done", key: "a", warning: "Past 80%" });
    expect(list[0]).toMatchObject({ status: "done", message: "Past 80%" });
    list = uploads(list, { type: "failed", key: "b", message: "Too large." });
    expect(list[1]).toMatchObject({ status: "failed", message: "Too large." });
    expect(uploadPercent(list[1] as Upload)).toBe(100);
    // Progress after the end changes nothing.
    expect(uploads(list, { type: "progress", key: "b", loaded: 1, total: 2 })).toEqual(list);
    expect(uploads(list, { type: "dismiss", key: "a" })).toHaveLength(1);
  });
});

describe("the project forms", () => {
  test("a project needs a one-line name of at most 100 characters", () => {
    expect(parse(projectSchema, { name: " ", icon: "", instructions: "" })).toEqual({
      errors: { name: "Project name is required." },
    });
    expect(parse(projectSchema, { name: "a\nb", icon: "", instructions: "" })).toEqual({
      errors: { name: "Project name must be a single line." },
    });
    expect(parse(projectSchema, { name: "x".repeat(101), icon: "", instructions: "" })).toEqual({
      errors: { name: "Project name must be at most 100 characters." },
    });
    expect(
      parse(projectSchema, { name: " Research ", icon: " 📚 ", instructions: " Be brief. " })
    ).toEqual({ value: { name: "Research", icon: "📚", instructions: "Be brief." } });
  });

  test("the icon is empty or one emoji, by core's rule", () => {
    for (const icon of ["📁", "👨‍👩‍👧‍👦", "🇩🇪", "1️⃣", " 🚀 ", ""]) {
      expect(isIcon(icon)).toBe(true);
    }
    for (const icon of ["ab", "x", "🚀 🚀", "🚀".repeat(11), "a\u0007é"]) {
      expect(isIcon(icon)).toBe(false);
    }
    expect(parse(projectSchema, { name: "A", icon: "ab", instructions: "" })).toEqual({
      errors: { icon: "Choose one emoji as the icon." },
    });
  });

  test("the panels send only what changed", () => {
    expect(parse(identitySchema(project), { name: "Research", icon: "📚" })).toEqual({ value: {} });
    expect(parse(identitySchema(project), { name: "Papers", icon: "" })).toEqual({
      value: { name: "Papers", icon: "" },
    });
    expect(instructionsBody(project, " Answer briefly. ")).toEqual({});
    expect(instructionsBody(project, "")).toEqual({ instructions: "" });
    expect(modelBody(project, null, null)).toEqual({});
    expect(modelBody({ ...project, defaultModelId: MODEL, reasoning: "high" }, null, null)).toEqual(
      { defaultModelId: null, reasoning: null }
    );
    expect(toolsBody({ ...project, toolServerIds: [SERVER, OTHER] }, [OTHER, SERVER], [])).toEqual(
      {}
    );
    expect(toolsBody(project, [SERVER], ["schedules"])).toEqual({
      toolServerIds: [SERVER],
      builtinTools: ["schedules"],
    });
  });

  test("labels a project with its icon", () => {
    expect(projectLabel(project)).toBe("📚 Research");
    expect(projectLabel({ ...project, icon: "" })).toBe("Research");
  });
});

describe("the model picker's rules", () => {
  const thinking = model("0196f6a4-7c1e-7d2a-9b3c-4d5e6f708102", "Thinker", {
    providerId: OTHER,
    providerName: "Other",
    contextWindow: 200_000,
    capabilities: {
      ...capabilities,
      vision: true,
      reasoning: true,
      reasoningLevels: ["low", "high"],
      defaultReasoning: "high",
    },
  });

  test("groups usable models by connection", () => {
    const groups = groupModels([
      model(MODEL, "GPT"),
      thinking,
      model("0196f6a4-7c1e-7d2a-9b3c-4d5e6f708103", "Old", { available: false }),
      model("0196f6a4-7c1e-7d2a-9b3c-4d5e6f708104", "Second"),
    ]);
    expect(groups.map((group) => [group.provider, group.models.map((m) => m.displayName)])).toEqual(
      [
        ["Mock", ["GPT", "Second"]],
        ["Other", ["Thinker"]],
      ]
    );
  });

  test("offers the reported reasoning levels, all four when none are reported", () => {
    expect(reasoningLevels(model(MODEL, "GPT"))).toEqual([]);
    expect(reasoningLevels(thinking)).toEqual(["low", "high"]);
    const unreported = { ...thinking.capabilities, reasoningLevels: [] };
    expect(reasoningLevels({ ...thinking, capabilities: unreported })).toEqual([
      "off",
      "low",
      "medium",
      "high",
    ]);
    expect(levelLabel(null, thinking)).toBe("Model default (high)");
    expect(levelLabel(null)).toBe("Model default");
    expect(levelLabel("low")).toBe("Low");
    expect(fitLevel(thinking, "medium")).toBeNull();
    expect(fitLevel(thinking, "low")).toBe("low");
    expect(fitLevel(model(MODEL, "GPT"), "low")).toBeNull();
  });

  test("summarizes flags and the context window", () => {
    expect(modelSummary(thinking)).toBe("Images, Tools, Reasoning · 200K context");
    expect(
      modelSummary(model(MODEL, "Plain", { capabilities: { ...capabilities, tools: false } }))
    ).toBe("Text only");
  });

  test("a new chat starts with the project's model, then the latest chat's, then the first", () => {
    const list = [model(MODEL, "GPT"), thinking];
    expect(startModel(list, thinking.id, MODEL)?.id).toBe(thinking.id);
    expect(startModel(list, null, "gone", MODEL)?.id).toBe(MODEL);
    expect(startModel(list, undefined, null)?.id).toBe(MODEL);
    expect(startModel([])).toBeUndefined();
  });
});

const user: UserResponse = {
  id: "0195d6a4-1b2c-7d3e-8f40-123456789abc",
  username: "ada",
  displayName: "Ada",
  role: "user",
  disabled: false,
  createdAt: AT,
  updatedAt: AT,
};

describe("the project screens", () => {
  function render(entry: string, data: [readonly unknown[], unknown][]) {
    const client = new QueryClient();
    setToken("token");
    client.setQueryData(["auth", "me", "token"], user);
    for (const [key, value] of data) client.setQueryData(key, value);
    return renderToString(
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={[entry]}>
          <Routes>
            <Route path={path.projects} element={<ProjectsPage />} />
            <Route path={path.projectNew} element={<ProjectPage />} />
            <Route path={path.project} element={<ProjectPage />} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );
  }

  const details: ProjectDetailsResponse = {
    ...project,
    defaultModelId: MODEL,
    toolServerIds: [SERVER],
    capacity: { used: 120_000, window: 100_000, level: "over" },
  };

  test("lists projects with their icon, pin, and instructions", () => {
    const html = render(path.projects, [
      [
        keys.projectList({}),
        [
          { ...project, pinned: true },
          { ...project, id: OTHER, icon: "" },
        ],
      ],
    ]);
    expect(html).toContain("Research");
    expect(html).toContain("📚");
    expect(html).toContain("Pinned");
    expect(html).toContain("Answer briefly.");
    expect(html).toContain(`href="/projects/${PROJECT}"`);
    expect(html).toContain(`href="${path.projectNew}"`);
  });

  test("without projects it says what a project is for", () => {
    expect(render(path.projects, [[keys.projectList({}), []]])).toContain("No projects yet");
  });

  test("the new project form asks for a name, an icon, and instructions", () => {
    const html = render(path.projectNew, []);
    expect(html).toContain("New Project");
    expect(html).toContain('name="name"');
    expect(html).toContain('name="icon"');
    expect(html).toContain('name="instructions"');
  });

  test("a project shows its composer, chats, instructions, files, model, and tools", () => {
    const html = render(`/projects/${PROJECT}`, [
      [keys.project(PROJECT), details],
      [
        keys.projectFiles(PROJECT),
        [
          file("big.pdf", 100_000, { mediaType: "application/pdf", pageCount: 33 }),
          file("notes.md", 20_000, { included: false, mediaType: "text/markdown" }),
        ],
      ],
      [keys.models, [model(MODEL, "GPT", { contextWindow: 100_000 })]],
      [
        keys.toolServers,
        [
          {
            id: SERVER,
            name: "Search",
            url: "https://mcp.example.com",
            enabled: true,
            approval: "ask",
            slug: "search",
            credentials: "missing",
            protocolVersion: null,
            toolModes: {},
            tools: [],
            toolsFetchedAt: null,
            reviewRequired: false,
            createdAt: AT,
            updatedAt: AT,
          },
        ],
      ],
      [
        keys.chatList({ projectId: PROJECT }),
        { pages: [{ items: [], next: null }], pageParams: [null] },
      ],
    ]);
    expect(html).toContain("📚 Research");
    expect(html).toContain("Start a chat in Research");
    expect(html).toContain("No chats in this project yet.");
    expect(html).toContain("Answer briefly.");
    expect(html).toContain("120K of 100K tokens (120%)");
    expect(html).toContain("Sending is blocked");
    expect(html).toContain("big.pdf (100K)");
    expect(html).toContain("PDF · 33 pages");
    expect(html).toContain("Include notes.md in context");
    expect(html).toContain("Upload files");
    expect(html).toContain("Default model");
    expect(html).toContain('<optgroup label="Mock">');
    expect(html).toContain("Search");
    expect(html).toContain("Schedules");
    expect(html).toContain('aria-label="Delete project"');
    expect(html).toContain('role="tablist"');
  });
});
