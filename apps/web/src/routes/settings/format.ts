// Formatting shared by the Settings screens. Pure functions, so tests pass a fixed `now`.

const relative = new Intl.RelativeTimeFormat("en", { numeric: "auto" });

const STEPS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["minute", 60],
  ["hour", 60 * 60],
  ["day", 24 * 60 * 60],
];

/**
 * How long ago `iso` was, in words ("just now", "5 minutes ago", "yesterday"); a date once it is
 * a month old. A time in the future reads "just now" (clocks drift).
 */
export function ago(iso: string, now: number = Date.now()): string {
  const at = Date.parse(iso);
  const seconds = Math.floor((now - at) / 1000);
  if (Number.isNaN(at)) return "";
  if (seconds < 60) return "just now";
  if (seconds >= 30 * 24 * 60 * 60) return date(iso);
  let unit: Intl.RelativeTimeFormatUnit = "minute";
  let size = 60;
  for (const [name, length] of STEPS) {
    if (seconds >= length) {
      unit = name;
      size = length;
    }
  }
  return relative.format(-Math.floor(seconds / size), unit);
}

/** A date and time in the reader's locale and time zone ("Nov 27, 2026, 9:14 AM"). */
export function dateTime(iso: string): string {
  return new Date(iso).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

/** A date in the reader's locale ("Nov 27, 2026"). */
export function date(iso: string): string {
  return new Date(iso).toLocaleDateString(undefined, { dateStyle: "medium" });
}

/** "1 chat", "3 chats". */
export function plural(count: number, noun: string, many = `${noun}s`): string {
  return `${count} ${count === 1 ? noun : many}`;
}
