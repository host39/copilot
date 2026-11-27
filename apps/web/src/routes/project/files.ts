// A project's files: the capacity bar (spec, Project Page), file details, and the uploads in
// progress. Pure functions, so tests check every rule without a browser.

import type { Capacity, CapacityLevel, FileResponse } from "@copilot/contracts";
import { shortTokens } from "@web/routes/settings/provider/models";

/** Past this share of the context window, uploads warn (core's `CONTEXT_WARN_RATIO`). */
export const WARN_RATIO = 0.8;

/** How many of the largest included files the "over" message names. */
const LARGEST_NAMED = 3;

/** Where `used` tokens stand against `window`, by core's rule (`Capacity::level`). */
export function capacityLevel(used: number, window: number | null): CapacityLevel {
  if (window === null || window <= 0) return "unknown";
  const ratio = used / window;
  if (ratio > 1) return "over";
  if (ratio > WARN_RATIO) return "warn";
  return "fits";
}

/** The capacity after `tokens` more are included (an upload, or a file turned on). */
export function capacityAfter(capacity: Capacity, tokens: number): Capacity {
  const used = Math.max(0, capacity.used + tokens);
  return { used, window: capacity.window, level: capacityLevel(used, capacity.window) };
}

/**
 * Included tokens the page knows of: the server's last capacity (`from`) plus the uploads that
 * finished since, so uploads that run at once each warn against the total before them.
 */
export type Tally = { from: Capacity; used: number };

/** The tally for the server's `capacity`: kept while it is the same answer, else started over. */
export function tally(current: Tally | null, capacity: Capacity): Tally {
  return current?.from === capacity ? current : { from: capacity, used: capacity.used };
}

/** The tally's capacity, and what an upload of `tokens` adds to it: the warning and the tally after. */
export function addUpload(current: Tally, tokens: number): { warning: string | null; next: Tally } {
  const window = current.from.window;
  const before: Capacity = {
    used: current.used,
    window,
    level: capacityLevel(current.used, window),
  };
  return {
    warning: uploadWarning(before, tokens),
    next: { from: current.from, used: current.used + tokens },
  };
}

/** The share of the window in use, as a whole percent; `null` without a window. */
export function percent(capacity: Capacity): number | null {
  if (capacity.window === null || capacity.window <= 0) return null;
  return Math.round((capacity.used / capacity.window) * 100);
}

/** The bar's label ("12K of 128K tokens (9%)", or "About 12K tokens" without a window). */
export function capacityText(capacity: Capacity): string {
  const share = percent(capacity);
  if (capacity.window === null || share === null) {
    return `About ${shortTokens(capacity.used)} tokens`;
  }
  return `${shortTokens(capacity.used)} of ${shortTokens(capacity.window)} tokens (${share}%)`;
}

/** The largest included files, largest first (core's `capacity::largest`). */
export function largest(files: FileResponse[], limit = LARGEST_NAMED): FileResponse[] {
  return files
    .filter((file) => file.included)
    .sort((a, b) => b.tokenEstimate - a.tokenEstimate)
    .slice(0, limit);
}

/** What the capacity bar says under it, by level; `null` when nothing needs saying. */
export function capacityNote(
  capacity: Capacity,
  files: FileResponse[],
  hasModel: boolean
): string | null {
  switch (capacity.level) {
    case "unknown":
      return hasModel
        ? "The default model's context window is unknown. Set it in Settings, Providers, to see how much fits."
        : "Choose a default model to see how much of its context window the files fill.";
    case "warn":
      return "The included files fill more than 80% of the default model's context window.";
    case "over": {
      const names = largest(files).map(
        (file) => `${file.name} (${shortTokens(file.tokenEstimate)})`
      );
      return `Sending is blocked: the included files do not fit the default model's context window. Turn off or remove some, starting with the largest: ${names.join(", ")}.`;
    }
    default:
      return null;
  }
}

const UNITS = ["KB", "MB", "GB"] as const;

/** A byte count for people ("512 bytes", "1.5 MB"). */
export function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} ${bytes === 1 ? "byte" : "bytes"}`;
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${Number(value.toFixed(value < 10 ? 1 : 0))} ${UNITS[unit]}`;
}

/** A file's details under its name ("PDF · 12 pages · 1.2 MB · about 36K tokens"). */
export function fileDetails(file: FileResponse): string {
  const kind = file.mediaType === "application/pdf" ? "PDF" : kindName(file.mediaType);
  const parts = [kind];
  if (file.pageCount !== null)
    parts.push(`${file.pageCount} ${file.pageCount === 1 ? "page" : "pages"}`);
  parts.push(size(file.sizeBytes), `about ${shortTokens(file.tokenEstimate)} tokens`);
  return parts.join(" · ");
}

function kindName(mediaType: string): string {
  if (mediaType.startsWith("image/")) return "Image";
  if (mediaType.startsWith("audio/")) return "Audio";
  if (mediaType === "text/markdown") return "Markdown";
  return "Text";
}

/** One upload on the project page. */
export type Upload = {
  /** A local id, unique among the page's uploads. */
  key: string;
  name: string;
  loaded: number;
  total: number;
  status: "uploading" | "done" | "failed";
  /** Why it failed, or the capacity warning it caused. */
  message?: string;
};

export type UploadAction =
  | { type: "start"; key: string; name: string; total: number }
  | { type: "progress"; key: string; loaded: number; total: number }
  | { type: "done"; key: string; warning?: string }
  | { type: "failed"; key: string; message: string }
  | { type: "dismiss"; key: string };

/** The uploads after `action`. Finished uploads stay listed until dismissed or replaced. */
export function uploads(list: Upload[], action: UploadAction): Upload[] {
  switch (action.type) {
    case "start":
      return [
        ...list.filter((item) => item.key !== action.key),
        { key: action.key, name: action.name, loaded: 0, total: action.total, status: "uploading" },
      ];
    case "progress":
      return list.map((item) =>
        item.key === action.key && item.status === "uploading"
          ? { ...item, loaded: Math.min(action.loaded, action.total), total: action.total }
          : item
      );
    case "done":
      // A clean upload leaves the list: the file shows in the project's files instead.
      return action.warning
        ? list.map((item) =>
            item.key === action.key
              ? { ...item, loaded: item.total, status: "done", message: action.warning }
              : item
          )
        : list.filter((item) => item.key !== action.key);
    case "failed":
      return list.map((item) =>
        item.key === action.key ? { ...item, status: "failed", message: action.message } : item
      );
    case "dismiss":
      return list.filter((item) => item.key !== action.key);
  }
}

/** An upload's progress as a whole percent. */
export function uploadPercent(upload: Upload): number {
  if (upload.total <= 0) return upload.status === "uploading" ? 0 : 100;
  return Math.round((upload.loaded / upload.total) * 100);
}

/**
 * The warning an upload earns: it took the project past 80% of the window, or past 100%. `null`
 * when the bar was already there, or has no window.
 */
export function uploadWarning(before: Capacity, tokens: number): string | null {
  const after = capacityAfter(before, tokens);
  if (after.level === before.level) return null;
  if (after.level === "over") {
    return "The included files no longer fit the default model's context window, so sending is blocked.";
  }
  if (after.level === "warn") {
    return `The included files now fill ${percent(after)}% of the default model's context window.`;
  }
  return null;
}
