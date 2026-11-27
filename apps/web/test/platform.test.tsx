// Platform behavior and the tools menu: the desktop shortcuts and their list, the Android back
// button, the chat list beneath a deep link on phones, closing the topmost dialog, and which tools
// a chat uses with what each change stores.

import type { ProjectResponse, ToolServerResponse } from "@copilot/contracts";
import { describe, expect, test } from "@rstest/core";
import { draftBody, retryBody } from "@web/components/Chat/StartChat";
import { ToolsMenu } from "@web/components/Chat/ToolsMenu";
import {
  builtinOn,
  serverRows,
  setBuiltin,
  setServer,
  TOOLS_OFF_NOTE,
  toolsWarning,
} from "@web/components/Chat/tools";
import {
  backAction,
  closeTop,
  NARROW_QUERY,
  needsHome,
  SHORTCUTS,
  shortcut,
} from "@web/components/Layout/platform";
import { ShortcutsDialog } from "@web/components/Layout/ShortcutsDialog";
import { renderToString } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";

const AT = "2026-11-26T10:00:00Z";
const PROJECT = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708200";
const MODEL = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708101";
const WEATHER = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708300";
const FILES = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708301";
const OFF = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708302";

/** A key press with nothing held unless `extra` says so. */
function press(key: string, extra: Partial<Parameters<typeof shortcut>[0]> = {}) {
  return {
    key,
    code: "",
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    altKey: false,
    isComposing: false,
    defaultPrevented: false,
    ...extra,
  };
}

describe("desktop shortcuts", () => {
  test("Ctrl+Shift+O, Ctrl+K, and Ctrl+/, or Cmd on a Mac", () => {
    expect(shortcut(press("O", { ctrlKey: true, shiftKey: true, code: "KeyO" }))).toBe("newChat");
    expect(shortcut(press("o", { metaKey: true, shiftKey: true }))).toBe("newChat");
    // Another layout's letter on the O key still counts by its place.
    expect(shortcut(press("ø", { ctrlKey: true, shiftKey: true, code: "KeyO" }))).toBe("newChat");
    expect(shortcut(press("k", { ctrlKey: true }))).toBe("search");
    expect(shortcut(press("K", { metaKey: true }))).toBe("search");
    expect(shortcut(press("/", { ctrlKey: true }))).toBe("help");
    // Some layouts need Shift to type `/`.
    expect(shortcut(press("/", { ctrlKey: true, shiftKey: true }))).toBe("help");
  });

  test("nothing without Ctrl, with Alt, while composing, once taken, or for other keys", () => {
    expect(shortcut(press("k"))).toBeNull();
    expect(shortcut(press("o", { shiftKey: true }))).toBeNull();
    expect(shortcut(press("o", { ctrlKey: true }))).toBeNull();
    expect(shortcut(press("k", { ctrlKey: true, shiftKey: true }))).toBeNull();
    expect(shortcut(press("k", { ctrlKey: true, altKey: true }))).toBeNull();
    expect(shortcut(press("k", { ctrlKey: true, isComposing: true }))).toBeNull();
    expect(shortcut(press("k", { ctrlKey: true, defaultPrevented: true }))).toBeNull();
    expect(shortcut(press("j", { ctrlKey: true }))).toBeNull();
  });

  test("the list names the spec's shortcuts", () => {
    expect(SHORTCUTS.map((item) => item.keys)).toEqual([
      "Ctrl+Shift+O",
      "Ctrl+K",
      "Esc",
      "Ctrl+/",
      "Enter",
    ]);
    const html = renderToString(<ShortcutsDialog open onClose={() => undefined} />);
    expect(html).toContain("Keyboard Shortcuts");
    for (const item of SHORTCUTS) expect(html).toContain(item.action);
    expect(html).toContain("<kbd");
  });
});

describe("the Android back button", () => {
  test("closes an overlay first, then goes back, then to the chat list, then leaves", () => {
    expect(backAction({ overlay: true, canGoBack: true, atHome: false })).toBe("close");
    expect(backAction({ overlay: true, canGoBack: false, atHome: true })).toBe("close");
    expect(backAction({ overlay: false, canGoBack: true, atHome: false })).toBe("back");
    expect(backAction({ overlay: false, canGoBack: true, atHome: true })).toBe("back");
    expect(backAction({ overlay: false, canGoBack: false, atHome: false })).toBe("home");
    expect(backAction({ overlay: false, canGoBack: false, atHome: true })).toBe("exit");
  });

  test("a phone opened on a deep link gets the chat list beneath it", () => {
    expect(needsHome({ index: 0, atHome: false, narrow: true })).toBe(true);
    expect(needsHome({ index: 0, atHome: true, narrow: true })).toBe(false);
    expect(needsHome({ index: 3, atHome: false, narrow: true })).toBe(false);
    expect(needsHome({ index: 0, atHome: false, narrow: false })).toBe(false);
    expect(NARROW_QUERY).toBe("(max-width: 63.99rem)");
  });

  test("closes the topmost dialog the way Escape does", () => {
    const calls: string[] = [];
    const dialog = (name: string, keep = false, request = false) => ({
      close: () => calls.push(`${name}.close`),
      dispatchEvent: (event: Event) => {
        calls.push(`${name}.${event.type}`);
        return !keep;
      },
      ...(request ? { requestClose: () => calls.push(`${name}.requestClose`) } : {}),
    });
    expect(closeTop([])).toBe(false);
    expect(closeTop([dialog("drawer"), dialog("sheet")])).toBe(true);
    expect(calls).toEqual(["sheet.cancel", "sheet.close"]);

    calls.length = 0;
    // A dialog that keeps itself open (an action is running) stays open.
    expect(closeTop([dialog("busy", true)])).toBe(true);
    expect(calls).toEqual(["busy.cancel"]);

    calls.length = 0;
    expect(closeTop([dialog("modern", false, true)])).toBe(true);
    expect(calls).toEqual(["modern.requestClose"]);
  });
});

function server(id: string, name: string, extra: Partial<ToolServerResponse> = {}) {
  return {
    id,
    name,
    slug: name.toLowerCase(),
    transport: "http",
    url: `https://${name.toLowerCase()}.example/mcp`,
    stdio: null,
    approval: "ask",
    toolModes: {},
    enabled: true,
    credentials: "missing",
    authMode: "none",
    oauth: null,
    protocolVersion: null,
    tools: null,
    toolsFetchedAt: null,
    reviewRequired: false,
    createdAt: AT,
    updatedAt: AT,
    ...extra,
  } satisfies ToolServerResponse;
}

const servers = [
  server(WEATHER, "Weather"),
  server(FILES, "Files", { reviewRequired: true }),
  server(OFF, "Retired", { enabled: false }),
];

const project: Pick<ProjectResponse, "id" | "toolServerIds" | "builtinTools"> = {
  id: PROJECT,
  toolServerIds: [WEATHER],
  builtinTools: ["schedules"],
};

describe("the tools menu", () => {
  test("lists the servers turned on in Settings, the project's checked and labeled", () => {
    expect(serverRows(servers, project, [])).toEqual([
      { id: WEATHER, name: "Weather", on: true, fromProject: true, review: false },
      { id: FILES, name: "Files", on: false, fromProject: false, review: true },
    ]);
    expect(
      serverRows(servers, project, [
        { serverId: WEATHER, enabled: false },
        { serverId: FILES, enabled: true },
      ]).map((row) => row.on)
    ).toEqual([false, true]);
    expect(serverRows(servers, null, []).every((row) => !row.on && !row.fromProject)).toBe(true);
    expect(serverRows(undefined, project, [])).toEqual([]);
  });

  test("a change stores an override only where the chat differs from its project", () => {
    expect(setServer(project, [], WEATHER, false)).toEqual([{ serverId: WEATHER, enabled: false }]);
    expect(setServer(project, [{ serverId: WEATHER, enabled: false }], WEATHER, true)).toEqual([]);
    expect(setServer(project, [], FILES, true)).toEqual([{ serverId: FILES, enabled: true }]);
    expect(setServer(project, [{ serverId: FILES, enabled: true }], FILES, false)).toEqual([]);
    expect(setServer(null, [], WEATHER, true)).toEqual([{ serverId: WEATHER, enabled: true }]);
    expect(setServer(project, [{ serverId: FILES, enabled: true }], WEATHER, false)).toEqual([
      { serverId: FILES, enabled: true },
      { serverId: WEATHER, enabled: false },
    ]);
  });

  test("built-in tools follow the project until the chat chooses its own", () => {
    expect(builtinOn(null, project)).toEqual(["schedules"]);
    expect(builtinOn([], project)).toEqual([]);
    expect(builtinOn(null, null)).toEqual([]);
    expect(setBuiltin([], "schedules", true)).toEqual(["schedules"]);
    expect(setBuiltin(["schedules"], "schedules", true)).toEqual(["schedules"]);
    expect(setBuiltin(["schedules"], "schedules", false)).toEqual([]);
  });

  test("the button warns when tools are on and the model cannot call them", () => {
    expect(toolsWarning(true, { tools: false })).toBe(true);
    expect(toolsWarning(true, { tools: true })).toBe(false);
    expect(toolsWarning(false, { tools: false })).toBe(false);
    expect(toolsWarning(true, undefined)).toBe(false);
    expect(TOOLS_OFF_NOTE).toContain("Tools are off for this model");
  });

  test("the button names the warning; the menu opens on demand", () => {
    const render = (warning: boolean) =>
      renderToString(
        <MemoryRouter>
          <ToolsMenu
            servers={servers}
            project={project}
            choice={{ toolOverrides: [], builtinTools: null }}
            onChange={() => undefined}
            warning={warning}
          />
        </MemoryRouter>
      );
    expect(render(false)).toContain('aria-label="Tools"');
    expect(render(false)).toContain('aria-expanded="false"');
    expect(render(true)).toContain('aria-label="Tools (the model cannot call them)"');
  });

  test("a new chat sends only the tools its draft changed", () => {
    const none = { toolOverrides: [], builtinTools: null };
    expect(draftBody(MODEL, null, null, none)).toEqual({ modelId: MODEL });
    expect(
      draftBody(MODEL, project, "low", {
        toolOverrides: [{ serverId: WEATHER, enabled: false }],
        builtinTools: [],
      })
    ).toEqual({
      modelId: MODEL,
      projectId: PROJECT,
      reasoning: "low",
      toolOverrides: [{ serverId: WEATHER, enabled: false }],
      builtinTools: [],
    });
  });

  test("sending again after a failed first send brings the created chat in line with the draft", () => {
    // Every choice is sent, so one changed back to its default also reaches the chat.
    const none = { toolOverrides: [], builtinTools: null };
    expect(retryBody(MODEL, null, none, false)).toEqual({
      modelId: MODEL,
      reasoning: null,
      toolOverrides: [],
      builtinTools: null,
      voiceReplies: false,
    });
    expect(
      retryBody(MODEL, "high", { toolOverrides: [], builtinTools: ["schedules"] }, true)
    ).toEqual({
      modelId: MODEL,
      reasoning: "high",
      toolOverrides: [],
      builtinTools: ["schedules"],
      voiceReplies: true,
    });
  });
});
