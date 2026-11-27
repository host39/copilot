// Schedules: the request helpers, the presets and the cron rules they write and read back, the
// form's checks and bodies, prefilling from a chat or a project, what the Scheduled list says, the
// tools a run would use, the `schedule_prompt` approval card, and the screens.

import type {
  ChatResponse,
  MessageResponse,
  ModelResponse,
  ProjectResponse,
  ScheduleResponse,
  ToolServerResponse,
  UserResponse,
} from "@copilot/contracts";
import { afterEach, describe, expect, test } from "@rstest/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { resetTransport, setTransport } from "@web/api/client";
import { keys } from "@web/api/routes/keys";
import {
  createSchedule,
  deleteSchedule,
  fetchSchedule,
  fetchSchedules,
  previewSchedule,
  runSchedule,
  updateSchedule,
} from "@web/api/routes/schedules";
import { clearToken, setToken } from "@web/auth/token";
import { ApprovalPanel } from "@web/components/Chat/ToolCard";
import { ScheduleProposal } from "@web/components/Schedule/Proposal";
import type { ToolCall } from "@web/routes/chat/stream";
import path from "@web/routes/path";
import { newScheduleLink } from "@web/routes/schedule/draft";
import SchedulePage from "@web/routes/schedule/page";
import {
  blankValues,
  chatValues,
  checkValues,
  createBody,
  DEFAULT_RULE,
  deviceZone,
  formatRun,
  projectValues,
  type Rule,
  readProposal,
  readRule,
  ruleErrors,
  type ScheduleValues,
  scheduleContext,
  scheduleStatus,
  scheduleValues,
  startValues,
  targetLabel,
  targetTools,
  toCron,
  toolNames,
  unreadRuns,
  updateBody,
  zoneList,
} from "@web/routes/schedule/rules";
import ScheduledPage from "@web/routes/scheduled/page";
import type { ReactNode } from "react";
import { renderToString } from "react-dom/server";
import { MemoryRouter, Route, Routes } from "react-router-dom";

const AT = "2026-11-26T10:00:00Z";
const SCHEDULE = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708400";
const PROJECT = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708200";
const MODEL = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708101";
const OTHER_MODEL = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708102";
const SERVER = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708300";
const CHAT = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708500";

type Call = { url: string; method: string; body: unknown };
let calls: Call[] = [];

function serve(answer: () => Response): void {
  setTransport({
    server: () => "",
    token: () => "token",
    fetch: async (input, init) => {
      calls.push({
        url: input,
        method: init?.method ?? "GET",
        body: typeof init?.body === "string" ? JSON.parse(init.body) : init?.body,
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

afterEach(() => {
  calls = [];
  clearToken();
  resetTransport();
});

const schedule: ScheduleResponse = {
  id: SCHEDULE,
  projectId: null,
  modelId: MODEL,
  sourceChatId: null,
  name: "Morning news",
  cron: "0 7 * * 1-5",
  timezone: "Europe/Berlin",
  description: "At 07:00 on Monday through Friday",
  prompt: "Summarize the news.",
  instructions: "Be brief.",
  reasoning: null,
  onAsk: "deny",
  createdBy: "user",
  enabled: true,
  nextRunAt: "2026-11-27T06:00:00Z",
  lastRunAt: null,
  lastStatus: null,
  lastError: null,
  consecutiveFailures: 0,
  disabledReason: null,
  createdAt: AT,
  updatedAt: AT,
};

const project: ProjectResponse = {
  id: PROJECT,
  name: "Research",
  icon: "📚",
  instructions: "",
  defaultModelId: OTHER_MODEL,
  reasoning: null,
  builtinTools: [],
  toolServerIds: [SERVER],
  pinned: false,
  createdAt: AT,
  updatedAt: AT,
};

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

function server(extra: Partial<ToolServerResponse> = {}): ToolServerResponse {
  const tool = (name: string, title: string | null = null) => ({
    name,
    title,
    description: "",
    inputSchema: { type: "object" },
    readOnly: false,
  });
  return {
    id: SERVER,
    name: "Weather",
    slug: "weather",
    url: "https://weather.example/mcp",
    approval: "ask",
    toolModes: { forecast: "auto", alerts: "off" },
    enabled: true,
    credentials: "missing",
    protocolVersion: "2025-06-18",
    tools: [tool("forecast", "Forecast"), tool("alerts"), tool("history")],
    toolsFetchedAt: AT,
    reviewRequired: false,
    createdAt: AT,
    updatedAt: AT,
    ...extra,
  };
}

const user: UserResponse = {
  id: "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708001",
  username: "admin",
  displayName: "Admin",
  role: "administrator",
  disabled: false,
  createdAt: AT,
  updatedAt: AT,
};

describe("the schedule request helpers", () => {
  test("list, read, create, change, delete, run, and preview", async () => {
    serve(() => json({ items: [schedule], total: 1 }));
    expect(await fetchSchedules({ projectId: PROJECT })).toEqual([schedule]);
    expect(calls[0]).toEqual({
      url: `/api/schedules?projectId=${PROJECT}`,
      method: "GET",
      body: undefined,
    });

    calls = [];
    serve(() => json({ data: schedule }));
    expect((await fetchSchedule(SCHEDULE)).name).toBe("Morning news");
    expect(calls[0]?.url).toBe(`/api/schedules/${SCHEDULE}`);

    calls = [];
    const body = { name: "A", cron: "0 9 * * *", timezone: "UTC", prompt: "Hi.", modelId: MODEL };
    await createSchedule(body);
    expect(calls[0]).toEqual({ url: "/api/schedules", method: "POST", body });

    calls = [];
    await updateSchedule(SCHEDULE, { enabled: false });
    expect(calls[0]).toEqual({
      url: `/api/schedules/${SCHEDULE}`,
      method: "PATCH",
      body: { enabled: false },
    });

    calls = [];
    serve(() => new Response(null, { status: 204 }));
    await deleteSchedule(SCHEDULE);
    expect(calls[0]).toMatchObject({ url: `/api/schedules/${SCHEDULE}`, method: "DELETE" });

    calls = [];
    serve(() => json({ data: { chatId: CHAT } }));
    expect(await runSchedule(SCHEDULE)).toBe(CHAT);
    expect(calls[0]).toMatchObject({ url: `/api/schedules/${SCHEDULE}/run`, method: "POST" });

    calls = [];
    const preview = { description: "At 09:00", timezone: "UTC", nextRuns: [AT, AT, AT] };
    serve(() => json({ data: preview }));
    expect(await previewSchedule({ cron: "0 9 * * *", timezone: "UTC" })).toEqual(preview);
    expect(calls[0]).toEqual({
      url: "/api/schedules/preview",
      method: "POST",
      body: { cron: "0 9 * * *", timezone: "UTC" },
    });
  });

  test("the new schedule link carries a project or a chat", () => {
    expect(newScheduleLink()).toBe("/scheduled/new");
    expect(newScheduleLink({ projectId: PROJECT })).toBe(`/scheduled/new?project=${PROJECT}`);
    expect(newScheduleLink({ chatId: CHAT })).toBe(`/scheduled/new?chat=${CHAT}`);
  });
});

describe("presets and cron rules", () => {
  const rule = (change: Partial<Rule>): Rule => ({ ...DEFAULT_RULE, ...change });

  test("each preset writes its cron rule", () => {
    expect(toCron(rule({ preset: "hourly", minute: 15 }))).toBe("15 * * * *");
    expect(toCron(rule({ preset: "daily", time: "07:30" }))).toBe("30 7 * * *");
    expect(toCron(rule({ preset: "weekdays", time: "09:00" }))).toBe("0 9 * * 1-5");
    expect(toCron(rule({ preset: "weekly", time: "18:05", weekday: 5 }))).toBe("5 18 * * 5");
    expect(toCron(rule({ preset: "monthly", time: "00:00", day: 31 }))).toBe("0 0 31 * *");
    expect(toCron(rule({ preset: "custom", cron: "  */30  9-17 * *   1-5 " }))).toBe(
      "*/30 9-17 * * 1-5"
    );
  });

  test("a saved rule reads back as the preset that writes it", () => {
    for (const preset of ["hourly", "daily", "weekdays", "weekly", "monthly"] as const) {
      const written = toCron(rule({ preset, time: "06:45", minute: 45, weekday: 3, day: 12 }));
      const read = readRule(written);
      expect(read.preset).toBe(preset);
      expect(toCron(read)).toBe(written);
    }
    expect(readRule("0 7 * * 1-5")).toMatchObject({ preset: "weekdays", time: "07:00" });
    expect(readRule("0 9 * * 7")).toMatchObject({ preset: "weekly", weekday: 0 });
  });

  test("anything a preset cannot write stays custom, as typed", () => {
    for (const cron of [
      "*/15 * * * *",
      "0 9-17 * * *",
      "0 9 1 1 *",
      "0 9 1 * 1",
      "0 24 * * *",
      "0 9 * * 1,3",
      "0 9 * *",
      "0 * 1 * *",
    ]) {
      expect(readRule(cron)).toMatchObject({ preset: "custom", cron });
    }
  });

  test("the rule is checked before the server sees it", () => {
    expect(ruleErrors(rule({ preset: "custom", cron: " " }))).toEqual({
      cron: "Enter a cron rule.",
    });
    expect(ruleErrors(rule({ preset: "custom", cron: "0 9 * *" })).cron).toContain("5 fields");
    expect(ruleErrors(rule({ preset: "custom", cron: "0 9 * * *" }))).toEqual({});
    expect(ruleErrors(rule({ preset: "daily", time: "" })).time).toContain("Enter a time");
    expect(ruleErrors(rule({ preset: "daily", time: "24:00" })).time).toBeDefined();
    expect(ruleErrors(rule({ preset: "hourly", minute: 60 })).time).toBeDefined();
    expect(ruleErrors(rule({ preset: "hourly", minute: 1.5 })).time).toBeDefined();
    expect(ruleErrors(rule({ preset: "hourly", minute: 0 }))).toEqual({});
    expect(ruleErrors(rule({ preset: "monthly", day: 0 }))).toEqual({
      day: "Choose a day from 1 to 31.",
    });
    expect(ruleErrors(rule({ preset: "monthly", day: 31 }))).toEqual({});
  });

  test("the time zone defaults to the device's, and the list holds UTC and the saved zone", () => {
    expect(deviceZone()).toBe(Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC");
    const zones = zoneList("Mars/Base", "UTC");
    expect(zones).toContain("UTC");
    expect(zones).toContain("Mars/Base");
    expect(new Set(zones).size).toBe(zones.length);
    expect([...zones].sort((a, b) => a.localeCompare(b, "en"))).toEqual(zones);
  });

  test("run times read in the given zone", () => {
    expect(formatRun("2026-11-27T06:00:00Z", { timeZone: "UTC", locale: "en-US" })).toBe(
      "Fri, Nov 27, 6:00 AM"
    );
    expect(formatRun("2026-11-27T06:00:00Z", { timeZone: "Europe/Berlin", locale: "en-US" })).toBe(
      "Fri, Nov 27, 7:00 AM"
    );
  });
});

describe("the schedule form", () => {
  const valid: ScheduleValues = {
    ...blankValues("UTC"),
    name: " Morning news ",
    prompt: " Summarize. ",
    modelId: MODEL,
  };

  test("checks core's rules and trims what passes", () => {
    const checked = checkValues(valid);
    expect(checked).toMatchObject({
      ok: true,
      cron: "0 9 * * *",
      values: { name: "Morning news", prompt: "Summarize." },
    });

    const failed = checkValues({
      ...valid,
      name: "",
      prompt: " ",
      modelId: "",
      timezone: "",
      rule: { ...DEFAULT_RULE, preset: "custom", cron: "" },
    });
    expect(failed).toEqual({
      ok: false,
      errors: {
        name: "Schedule name is required.",
        prompt: "Enter the prompt to run.",
        modelId: "Choose a model for this schedule.",
        timezone: "Choose a time zone, for example Europe/Berlin.",
        cron: "Enter a cron rule.",
      },
    });
    expect(checkValues({ ...valid, name: "a\nb" })).toMatchObject({
      errors: { name: "Schedule name must be a single line." },
    });
    expect(checkValues({ ...valid, name: "x".repeat(101) })).toMatchObject({
      errors: { name: "Schedule name must be at most 100 characters." },
    });
  });

  test("a new schedule sends instructions only without a project, and a level only when set", () => {
    expect(createBody({ ...valid, instructions: "Be brief." }, "0 9 * * *")).toEqual({
      name: valid.name,
      cron: "0 9 * * *",
      timezone: "UTC",
      prompt: valid.prompt,
      modelId: MODEL,
      onAsk: "deny",
      instructions: "Be brief.",
    });
    expect(
      createBody(
        {
          ...valid,
          projectId: PROJECT,
          instructions: "Ignored.",
          reasoning: "low",
          onAsk: "pause",
        },
        "0 9 * * *"
      )
    ).toEqual({
      name: valid.name,
      cron: "0 9 * * *",
      timezone: "UTC",
      prompt: valid.prompt,
      modelId: MODEL,
      onAsk: "pause",
      projectId: PROJECT,
      reasoning: "low",
    });
  });

  test("a saved schedule sends only what changed", () => {
    const values = scheduleValues(schedule);
    expect(values.rule.preset).toBe("weekdays");
    expect(updateBody(schedule, values, toCron(values.rule))).toEqual({});
    expect(
      updateBody(
        schedule,
        {
          ...values,
          name: "Evening news",
          timezone: "UTC",
          prompt: "Other.",
          instructions: "",
          modelId: OTHER_MODEL,
          reasoning: "high",
          onAsk: "pause",
        },
        "0 18 * * *"
      )
    ).toEqual({
      name: "Evening news",
      cron: "0 18 * * *",
      timezone: "UTC",
      prompt: "Other.",
      instructions: "",
      modelId: OTHER_MODEL,
      reasoning: "high",
      onAsk: "pause",
    });
    // A level goes back to the default with `null`; a project schedule never sends instructions.
    expect(updateBody({ ...schedule, reasoning: "low" }, values, schedule.cron)).toEqual({
      reasoning: null,
    });
    const inProject = { ...schedule, projectId: PROJECT, instructions: "" };
    expect(
      updateBody(inProject, { ...scheduleValues(inProject), instructions: "x" }, schedule.cron)
    ).toEqual({});
    // A schedule whose model was deleted keeps it unset until one is chosen.
    const orphan = { ...schedule, modelId: null };
    expect(updateBody(orphan, scheduleValues(orphan), schedule.cron)).toEqual({});
  });

  const chat: Pick<ChatResponse, "title" | "modelId" | "projectId" | "instructions" | "reasoning"> =
    {
      title: "News digest",
      modelId: MODEL,
      projectId: null,
      instructions: "Be brief.",
      reasoning: "medium",
    };
  const messages: Pick<MessageResponse, "role" | "parts">[] = [
    { role: "user", parts: [{ type: "text", text: "First." }] },
    { role: "assistant", parts: [{ type: "text", text: "Answer." }] },
    {
      role: "user",
      parts: [
        { type: "text", text: "Summarize the news." },
        {
          type: "file",
          fileId: CHAT,
          name: "a.txt",
          mediaType: "text/plain",
        },
      ],
    },
    { role: "assistant", parts: [{ type: "text", text: "Done." }] },
  ];

  test("Schedule this prompt takes the last user message, the model, and the chat's settings", () => {
    expect(chatValues(chat, messages, "UTC")).toEqual({
      ...blankValues("UTC"),
      name: "News digest",
      prompt: "Summarize the news.",
      modelId: MODEL,
      reasoning: "medium",
      projectId: null,
      instructions: "Be brief.",
    });
    const inProject = chatValues({ ...chat, projectId: PROJECT, title: "" }, messages, "UTC");
    expect(inProject).toMatchObject({
      projectId: PROJECT,
      instructions: "",
      name: "Summarize the news.",
    });
    expect(chatValues(chat, [], "UTC").prompt).toBe("");
  });

  test("a new schedule starts with the chat's model, else the project's, else the first usable", () => {
    const models = [model(MODEL, "Fast"), model(OTHER_MODEL, "Smart")];
    expect(startValues({ chat, messages, models, timezone: "UTC" }).modelId).toBe(MODEL);
    expect(
      startValues({
        chat: { ...chat, modelId: null },
        messages,
        project,
        models,
        timezone: "UTC",
      }).modelId
    ).toBe(OTHER_MODEL);
    expect(startValues({ project, models, timezone: "UTC" })).toMatchObject({
      projectId: PROJECT,
      modelId: OTHER_MODEL,
    });
    expect(projectValues(project, "UTC").modelId).toBe(OTHER_MODEL);
    expect(
      startValues({ models: [model(MODEL, "Off", { enabled: false }), model(OTHER_MODEL, "On")] })
        .modelId
    ).toBe(OTHER_MODEL);
    expect(startValues({ models: [] }).modelId).toBe("");
    expect(startValues({ models: [] }).timezone).toBe(deviceZone());
  });
});

describe("the Scheduled list", () => {
  test("says where each schedule stands, a pause with its reason", () => {
    expect(scheduleStatus(schedule)).toEqual({
      label: "Not run yet",
      tone: "neutral",
      detail: null,
    });
    expect(scheduleStatus({ ...schedule, lastStatus: "ok" }).label).toBe("Last run ok");
    expect(
      scheduleStatus({ ...schedule, lastStatus: "error", lastError: "The key was rejected." })
    ).toEqual({ label: "Last run failed", tone: "danger", detail: "The key was rejected." });
    expect(scheduleStatus({ ...schedule, lastStatus: "awaiting_approval" }).label).toBe(
      "Awaiting approval"
    );
    expect(scheduleStatus({ ...schedule, enabled: false }).label).toBe("Off");
    expect(
      scheduleStatus({ ...schedule, enabled: false, disabledReason: "Model removed" })
    ).toEqual({ label: "Paused", tone: "danger", detail: "Model removed" });
  });

  test("names the target and counts unread runs per schedule", () => {
    expect(targetLabel(null, [project])).toBe("Standalone");
    expect(targetLabel(PROJECT, [project])).toBe("📚 Research");
    expect(targetLabel(PROJECT, [{ ...project, icon: "" }])).toBe("Research");
    expect(targetLabel(PROJECT, undefined)).toBe("Project");
    const counts = unreadRuns([
      { scheduleId: SCHEDULE, unread: true },
      { scheduleId: SCHEDULE, unread: true },
      { scheduleId: SCHEDULE, unread: false },
      { scheduleId: null, unread: true },
      { scheduleId: PROJECT, unread: true },
    ]);
    expect(counts.get(SCHEDULE)).toBe(2);
    expect(counts.get(PROJECT)).toBe(1);
  });
});

describe("the tools a run would use", () => {
  test("a project's enabled servers, without off tools; a standalone schedule has none", () => {
    expect(targetTools(project, [server()])).toEqual([
      { server: "Weather", tool: "Forecast", mode: "auto" },
      { server: "Weather", tool: "history", mode: "ask" },
    ]);
    expect(targetTools(null, [server()])).toEqual([]);
    expect(targetTools({ toolServerIds: [] }, [server()])).toEqual([]);
    expect(targetTools(project, [server({ enabled: false })])).toEqual([]);
    expect(targetTools(project, [server({ tools: null })])).toEqual([]);
  });

  test("a server whose list changed asks for every tool until it is reviewed", () => {
    expect(
      targetTools(project, [server({ reviewRequired: true })]).map((tool) => tool.mode)
    ).toEqual(["ask", "ask", "ask"]);
    expect(toolNames(targetTools(project, [server()]))).toBe("Weather: Forecast, Weather: history");
  });
});

describe("the schedule_prompt approval card", () => {
  const call = (input: unknown, extra: Partial<ToolCall> = {}): ToolCall => ({
    type: "toolCall",
    callId: "call-1",
    serverId: null,
    toolName: "schedule_prompt",
    arguments: input as ToolCall["arguments"],
    complete: true,
    approval: "pending",
    denyReason: null,
    ...extra,
  });
  const input = { name: "Digest", cron: "0 9 * * *", timezone: "UTC", prompt: "News?" };

  test("reads the four fields, from JSON or text, and nothing else", () => {
    expect(readProposal(call(input))).toEqual(input);
    expect(readProposal(call(JSON.stringify(input)))).toEqual(input);
    expect(readProposal(call("{not json"))).toBeNull();
    expect(readProposal(call({ ...input, prompt: 3 }))).toBeNull();
    expect(readProposal(call(null))).toBeNull();
    expect(readProposal(call(input, { toolName: "cancel_schedule" }))).toBeNull();
    expect(readProposal(call(input, { serverId: SERVER }))).toBeNull();
  });

  test("the context names the chat's model, its project, and the tools that run without asking", () => {
    expect(scheduleContext(model(MODEL, "Fast"), project, [server()])).toEqual({
      model: "Fast",
      target: "📚 Research",
      autoTools: "Weather: Forecast",
    });
    expect(scheduleContext(undefined, null, [server()])).toEqual({
      model: null,
      target: "Standalone",
      autoTools: "",
    });
  });

  function wrap(node: ReactNode, preview?: unknown) {
    const client = new QueryClient();
    if (preview) client.setQueryData(keys.schedulePreview("0 9 * * *", "UTC"), preview);
    return renderToString(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
  }

  test("shows the prompt, model, target, and auto tools instead of the raw arguments", () => {
    const html = wrap(
      <ApprovalPanel
        calls={[call(input)]}
        onDecide={async () => true}
        schedule={{ model: "Fast", target: "📚 Research", autoTools: "Weather: Forecast" }}
      />
    );
    expect(html).toContain("The assistant wants to create the schedule");
    expect(html).toContain("Digest");
    expect(html).toContain("News?");
    expect(html).toContain("Fast");
    expect(html).toContain("📚 Research");
    expect(html).toContain("Weather: Forecast");
    expect(html).not.toContain("Arguments");
    // Built-in tools always ask, so there is no "Approve for this chat".
    expect(html).not.toContain("Approve for this chat");
  });

  test("the preview lists the rule in words and the next runs", () => {
    const html = wrap(
      <ScheduleProposal
        proposal={input}
        context={{ model: null, target: "Standalone", autoTools: "" }}
      />
    );
    expect(html).toContain("Reading the rule");
    expect(html).toContain("None");
    expect(html).toContain("This chat&#x27;s model");
  });
});

describe("the schedule screens", () => {
  function render(entry: string, data: [readonly unknown[], unknown][]) {
    const client = new QueryClient();
    setToken("token");
    client.setQueryData(["auth", "me", "token"], user);
    for (const [key, value] of data) client.setQueryData(key, value);
    return renderToString(
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={[entry]}>
          <Routes>
            <Route path={path.scheduled} element={<ScheduledPage />} />
            <Route path={path.scheduleNew} element={<SchedulePage />} />
            <Route path={path.schedule} element={<SchedulePage />} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );
  }

  test("the list shows each schedule's target, rule, state, switch, Run now, and runs", () => {
    const html = render(path.scheduled, [
      [
        keys.scheduleList({}),
        [
          schedule,
          {
            ...schedule,
            id: PROJECT,
            name: "Weekly digest",
            projectId: PROJECT,
            createdBy: "assistant",
            enabled: false,
            nextRunAt: null,
            disabledReason: "Failed 3 times in a row",
          },
        ],
      ],
      [keys.projectList({}), [project]],
      [
        keys.runs,
        {
          items: [
            {
              id: CHAT,
              projectId: null,
              modelId: MODEL,
              scheduleId: SCHEDULE,
              title: "Morning news · Nov 26",
              pinned: false,
              unread: true,
              awaitingApproval: false,
              createdAt: AT,
              updatedAt: AT,
            },
          ],
          next: null,
        },
      ],
    ]);
    expect(html).toContain("Morning news");
    expect(html).toContain("Standalone · At 07:00 on Monday through Friday");
    expect(html).toContain("📚 Research");
    expect(html).toContain("By assistant");
    expect(html).toContain("Paused");
    expect(html).toContain("Failed 3 times in a row");
    expect(html).toContain("No next run");
    expect(html).toContain("Run now");
    expect(html).toContain('role="switch"');
    expect(html).toContain(`href="/scheduled/${SCHEDULE}"`);
    expect(html).toContain('1<span class="sr-only"> unread</span>');
    expect(html).toContain('href="/scheduled/new"');
  });

  test("without schedules the list says what they are for", () => {
    expect(render(path.scheduled, [[keys.scheduleList({}), []]])).toContain("No schedules yet");
  });

  test("the new schedule form starts daily in the device's zone with the first usable model", () => {
    const html = render(path.scheduleNew, [
      [keys.models, [model(MODEL, "Fast")]],
      [keys.projectList({}), [project]],
    ]);
    expect(html).toContain("New Schedule");
    expect(html).toContain("Every day at");
    expect(html).toContain('value="09:00"');
    expect(html).toContain(`value="${deviceZone()}" selected=""`);
    expect(html).toContain(`value="${MODEL}" selected=""`);
    expect(html).toContain("Standalone");
    expect(html).toContain("Instructions");
    expect(html).toContain("Create schedule");
  });

  test("a project's new schedule asks what a run does when a tool asks", () => {
    const html = render(newScheduleLink({ projectId: PROJECT }), [
      [keys.models, [model(MODEL, "Fast"), model(OTHER_MODEL, "Smart")]],
      [keys.projectList({}), [project]],
      [
        keys.project(PROJECT),
        { ...project, capacity: { used: 0, window: null, level: "unknown" } },
      ],
      [keys.toolServers, [server()]],
    ]);
    expect(html).toContain(`value="${PROJECT}" selected=""`);
    expect(html).toContain(`value="${OTHER_MODEL}" selected=""`);
    expect(html).toContain("Deny and continue");
    expect(html).toContain("Pause until I approve");
    expect(html).toContain("Tools that ask: Weather: history.");
  });

  test("a saved schedule shows its state, the form with its rule, its runs, and Delete", () => {
    const html = render(`/scheduled/${SCHEDULE}`, [
      [keys.schedule(SCHEDULE), { ...schedule, createdBy: "assistant" }],
      [keys.models, [model(MODEL, "Fast")]],
      [keys.projectList({}), []],
    ]);
    expect(html).toContain("Morning news");
    expect(html).toContain("By assistant");
    expect(html).toContain("Weekdays at");
    expect(html).toContain('value="07:00"');
    expect(html).toContain('value="Europe/Berlin" selected=""');
    expect(html).toContain("Runs");
    expect(html).toContain("Delete");
    expect(html).toContain("Save");
  });
});
