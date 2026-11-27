// The composer: Enter per platform, attached files and the model's capabilities, uploads, when a
// message can go, and the field with Send, Stop, and attach.

import type { Capabilities } from "@copilot/contracts";
import { describe, expect, test } from "@rstest/core";
import { Composer } from "@web/components/Chat/Composer";
import {
  type Attachment,
  attachments,
  canSend,
  enterSends,
  mediaType,
  unreadable,
} from "@web/components/Chat/composer";
import { renderToString } from "react-dom/server";

const enter = { key: "Enter", shiftKey: false, isComposing: false, keyCode: 13 };

const capabilities: Capabilities = {
  vision: false,
  pdf: false,
  tools: true,
  audioIn: false,
  audioOut: false,
  reasoning: false,
  reasoningLevels: [],
  defaultReasoning: null,
};

function item(extra: Partial<Attachment> = {}): Attachment {
  return {
    key: "file-1",
    name: "photo.png",
    mediaType: "image/png",
    size: 2_000,
    status: "uploading",
    loaded: 0,
    ...extra,
  };
}

describe("Enter", () => {
  test("with a mouse or trackpad Enter sends and Shift+Enter adds a line", () => {
    expect(enterSends(enter, true)).toBe(true);
    expect(enterSends({ ...enter, shiftKey: true }, true)).toBe(false);
    expect(enterSends({ ...enter, key: "a" }, true)).toBe(false);
  });

  test("on touch screens Enter adds a line and only the Send button sends", () => {
    expect(enterSends(enter, false)).toBe(false);
  });

  test("Enter that confirms an input method's text never sends", () => {
    expect(enterSends({ ...enter, isComposing: true }, true)).toBe(false);
    // Safari ends the composition before the confirming keydown but still reports 229.
    expect(enterSends({ ...enter, keyCode: 229 }, true)).toBe(false);
  });
});

describe("attached files", () => {
  test("the media type comes from the browser, else from the extension", () => {
    expect(mediaType("notes.md", "text/markdown")).toBe("text/markdown");
    expect(mediaType("Scan.PDF", "")).toBe("application/pdf");
    expect(mediaType("photo.jpeg", "")).toBe("image/jpeg");
    expect(mediaType("voice.mp3", "")).toBe("audio/mpeg");
    expect(mediaType("main.rs", "")).toBe("");
    expect(mediaType("README", "")).toBe("");
  });

  test("a file the model cannot read gets a warning; text never does", () => {
    expect(unreadable("image/png", capabilities)).toBe("Images are not sent to this model");
    expect(unreadable("application/pdf", capabilities)).toBe("PDFs are not sent to this model");
    expect(unreadable("audio/wav", capabilities)).toBe("Audio is not sent to this model");
    expect(unreadable("text/plain", capabilities)).toBeNull();
    expect(unreadable("", capabilities)).toBeNull();
    const able = { ...capabilities, vision: true, pdf: true, audioIn: true };
    expect(unreadable("image/png", able)).toBeNull();
    expect(unreadable("application/pdf", able)).toBeNull();
    expect(unreadable("audio/wav", able)).toBeNull();
    // Without a model nothing is said.
    expect(unreadable("image/png", null)).toBeNull();
  });

  test("uploads move from progress to ready, or fail with a reason", () => {
    let list = attachments([], { type: "add", item: item() });
    list = attachments(list, { type: "progress", key: "file-1", loaded: 500 });
    expect(list[0]?.loaded).toBe(500);
    list = attachments(list, { type: "progress", key: "file-1", loaded: 9_999 });
    expect(list[0]?.loaded).toBe(2_000);
    list = attachments(list, {
      type: "ready",
      key: "file-1",
      fileId: "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708210",
      mediaType: "image/webp",
    });
    expect(list[0]).toMatchObject({
      status: "ready",
      fileId: "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708210",
      mediaType: "image/webp",
    });
    // A late progress event changes nothing once the upload is done.
    expect(attachments(list, { type: "progress", key: "file-1", loaded: 1 })).toEqual(list);

    const failed = attachments([item({ key: "file-2" })], {
      type: "failed",
      key: "file-2",
      error: "This file type is not supported.",
    });
    expect(failed[0]).toMatchObject({
      status: "failed",
      error: "This file type is not supported.",
    });
    expect(attachments(failed, { type: "remove", key: "file-2" })).toEqual([]);
    expect(attachments(list, { type: "clear" })).toEqual([]);
  });

  test("adding a file with a key already listed replaces it", () => {
    const list = attachments([item()], { type: "add", item: item({ name: "other.png" }) });
    expect(list.map((entry) => entry.name)).toEqual(["other.png"]);
  });
});

describe("sending", () => {
  test("a message needs text or a ready file, and waits for every upload", () => {
    expect(canSend("", [])).toBe(false);
    expect(canSend("  ", [])).toBe(false);
    expect(canSend("Hi", [])).toBe(true);
    expect(canSend("", [item({ status: "ready" })])).toBe(true);
    expect(canSend("Hi", [item()])).toBe(false);
    // A failed upload is left out; it alone sends nothing.
    expect(canSend("", [item({ status: "failed" })])).toBe(false);
    expect(canSend("Hi", [item({ status: "failed" })])).toBe(true);
  });
});

describe("the composer", () => {
  const send = async () => true;

  test("has the message field, attach, and Send", () => {
    const html = renderToString(<Composer onSend={send} placeholder="Ask anything" />);
    expect(html).toContain("<textarea");
    expect(html).toContain('placeholder="Ask anything"');
    expect(html).toContain("[field-sizing:content]");
    expect(html).toContain('aria-label="Attach files"');
    expect(html).toContain('type="file"');
    expect(html).toContain('aria-label="Send"');
    expect(html).not.toContain('aria-label="Stop"');
  });

  test("Send becomes Stop while an answer is written", () => {
    const html = renderToString(<Composer onSend={send} running onStop={() => undefined} />);
    expect(html).toContain('aria-label="Stop"');
    expect(html).not.toContain('aria-label="Send"');
  });

  test("a chat that cannot answer disables the field but keeps attach visible", () => {
    const html = renderToString(<Composer onSend={send} disabled />);
    expect(html).toMatch(/<textarea[^>]*disabled/);
    expect(html).toContain('aria-label="Attach files"');
  });
});
