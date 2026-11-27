// The composer's rules, apart from rendering: when Enter sends (spec, Platform Behavior), what an
// attached file is and whether the model reads it (spec, Capability Display), the attachments as
// they upload, and when a message can be sent.

import type { Capabilities } from "@copilot/contracts";

/** The parts of a `keydown` that decide what Enter does. */
export type EnterKey = {
  key: string;
  shiftKey: boolean;
  isComposing: boolean;
  /** `229` while an input method composes (Safari reports `isComposing` late). */
  keyCode: number;
};

/**
 * Whether a key sends the message. With a fine pointer (a mouse or trackpad), Enter sends and
 * Shift+Enter adds a line; on touch screens Enter adds a line and the Send button sends. Enter
 * that confirms an input method's text never sends.
 */
export function enterSends(event: EnterKey, finePointer: boolean): boolean {
  if (event.key !== "Enter" || event.shiftKey || !finePointer) return false;
  return !(event.isComposing || event.keyCode === 229);
}

/** Whether the device's main pointer is fine (`(pointer: fine)`): a mouse or trackpad. */
export function finePointer(): boolean {
  return typeof matchMedia === "function" && matchMedia("(pointer: fine)").matches;
}

const EXTENSION_TYPES: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  webp: "image/webp",
  gif: "image/gif",
  pdf: "application/pdf",
  wav: "audio/wav",
  mp3: "audio/mpeg",
};

/**
 * A picked file's media type: the browser's when it gives one, else from the name's extension
 * (the server checks the bytes either way). `""` when neither says.
 */
export function mediaType(name: string, type: string): string {
  if (type) return type;
  const extension = name.includes(".") ? (name.split(".").pop() ?? "").toLowerCase() : "";
  return EXTENSION_TYPES[extension] ?? "";
}

/**
 * The warning chip for a file the model cannot read ("Images are not sent to this model"), or
 * `null` when it can (text always goes inline). Without known capabilities nothing is said.
 */
export function unreadable(
  type: string,
  capabilities: Pick<Capabilities, "vision" | "pdf" | "audioIn"> | null | undefined
): string | null {
  if (!capabilities) return null;
  if (type.startsWith("image/") && !capabilities.vision) {
    return "Images are not sent to this model";
  }
  if (type === "application/pdf" && !capabilities.pdf) return "PDFs are not sent to this model";
  if (type.startsWith("audio/") && !capabilities.audioIn) return "Audio is not sent to this model";
  return null;
}

/** A file attached to the message being written. */
export type Attachment = {
  /** A local id, unique among the composer's attachments. */
  key: string;
  name: string;
  mediaType: string;
  size: number;
  /** Uploading to the chat, uploaded (or kept to upload with the first send), or failed. */
  status: "uploading" | "ready" | "failed";
  loaded: number;
  /** The stored file, once uploaded. */
  fileId?: string;
  /** Why the upload failed. */
  error?: string;
};

export type AttachmentAction =
  | { type: "add"; item: Attachment }
  | { type: "progress"; key: string; loaded: number }
  | { type: "ready"; key: string; fileId: string; mediaType: string }
  | { type: "failed"; key: string; error: string }
  | { type: "remove"; key: string }
  | { type: "clear" };

/** The attachments after `action`. */
export function attachments(list: Attachment[], action: AttachmentAction): Attachment[] {
  const change = (key: string, update: (item: Attachment) => Attachment) =>
    list.map((item) => (item.key === key ? update(item) : item));
  switch (action.type) {
    case "add":
      return [...list.filter((item) => item.key !== action.item.key), action.item];
    case "progress":
      return change(action.key, (item) =>
        item.status === "uploading" ? { ...item, loaded: Math.min(action.loaded, item.size) } : item
      );
    case "ready":
      return change(action.key, (item) => ({
        ...item,
        status: "ready",
        loaded: item.size,
        fileId: action.fileId,
        mediaType: action.mediaType,
      }));
    case "failed":
      return change(action.key, (item) => ({ ...item, status: "failed", error: action.error }));
    case "remove":
      return list.filter((item) => item.key !== action.key);
    case "clear":
      return [];
  }
}

/**
 * What becomes of a file whose upload finished: its chip turns ready, or, when the chip was
 * removed while the file uploaded (`attached` is false), the stored file is deleted (`discard`),
 * so it never stays on the chat unsent.
 */
export function uploaded(
  attached: boolean,
  key: string,
  saved: { id: string; mediaType: string }
): AttachmentAction | { discard: string } {
  if (!attached) return { discard: saved.id };
  return { type: "ready", key, fileId: saved.id, mediaType: saved.mediaType };
}

/**
 * Whether the message can go: it has text or a ready file, and no file is still uploading (a
 * failed one is left out, and its chip says why).
 */
export function canSend(text: string, list: Attachment[]): boolean {
  if (list.some((item) => item.status === "uploading")) return false;
  return text.trim() !== "" || list.some((item) => item.status === "ready");
}

/** How the composer's recording bar closed: Stop (or the cap, or a failure), or Cancel. */
export type RecordingEnd = "stop" | "cancel";

/**
 * Where focus goes when the recording bar closes, so a keyboard or screen reader user lands back
 * in the composer instead of at the top of the page: the message field after Stop, to write and
 * send with the clip, and the microphone after Cancel. On touch screens (`fine` false) the
 * microphone either way, so the on-screen keyboard does not open over the clip. Nowhere when
 * focus already moved on (`lost` false: it is not on the page's body).
 */
export function refocus(end: RecordingEnd, lost: boolean, fine: boolean): "field" | "mic" | null {
  if (!lost) return null;
  return end === "stop" && fine ? "field" : "mic";
}
