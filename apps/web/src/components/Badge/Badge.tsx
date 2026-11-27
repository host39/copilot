import type { ReactNode } from "react";

export type BadgeTone = "neutral" | "ok" | "warn" | "danger" | "accent";

const TONE_CLASS_MAP: Record<BadgeTone, string> = {
  neutral: "bg-raised text-muted",
  ok: "bg-ok/15 text-ok",
  warn: "bg-warn/15 text-warn",
  danger: "bg-danger/15 text-danger",
  accent: "bg-accent/15 text-accent",
};

/** A short status label ("Disabled", "Key missing"). Its text carries the meaning, not its color. */
export function Badge({ tone = "neutral", children }: { tone?: BadgeTone; children: ReactNode }) {
  return (
    <span
      className={`inline-flex shrink-0 items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium ${TONE_CLASS_MAP[tone]}`}
    >
      {children}
    </span>
  );
}
