import { useQuery } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { keys } from "@web/api/routes/keys";
import { previewSchedule } from "@web/api/routes/schedules";
import { Select } from "@web/components/Form/Select";
import { TextField } from "@web/components/Form/TextField";
import { useDebounced } from "@web/hooks/useDebounced";
import {
  deviceZone,
  formatRun,
  PRESETS,
  type Preset,
  type Rule,
  ruleErrors,
  toCron,
  WEEKDAYS,
  zoneList,
} from "@web/routes/schedule/rules";
import { type ReactNode, useMemo } from "react";

/** A preview stays fresh for a minute: the next runs move only as time passes. */
const PREVIEW_STALE_MS = 60_000;

/**
 * The rule in plain words and its next 3 runs (`POST /schedules/preview`), in the device's time,
 * read once the user pauses typing. The server's message shows when it refuses the rule.
 */
export function SchedulePreview({ cron, timezone }: { cron: string; timezone: string }) {
  // One string, so a render with the same rule never restarts the pause.
  const [settledCron = "", settledZone = ""] = useDebounced(`${cron}\n${timezone}`, 400).split(
    "\n"
  );
  const settled = { cron: settledCron, timezone: settledZone };
  const ready = settled.cron !== "" && settled.timezone !== "";
  const preview = useQuery({
    queryKey: keys.schedulePreview(settled.cron, settled.timezone),
    queryFn: ({ signal }) => previewSchedule(settled, signal),
    enabled: ready,
    retry: false,
    staleTime: PREVIEW_STALE_MS,
  });
  const local = deviceZone();

  let body: ReactNode;
  if (!ready) body = <p className="text-muted">Choose when it runs to see the next runs.</p>;
  else if (preview.isError) {
    body = <p className="text-danger">{describeError(preview.error).message}</p>;
  } else if (!preview.data) body = <p className="text-muted">Reading the rule...</p>;
  else {
    body = (
      <>
        <p className="font-medium">{preview.data.description}</p>
        <p className="mt-2 text-xs text-muted">
          {local === preview.data.timezone
            ? "Next runs"
            : `Next runs in your time (${local}); the rule follows ${preview.data.timezone}`}
        </p>
        <ul className="mt-1 space-y-0.5">
          {preview.data.nextRuns.map((run) => (
            <li key={run}>
              <time dateTime={run}>{formatRun(run)}</time>
            </li>
          ))}
        </ul>
      </>
    );
  }

  return (
    <div role="status" className="rounded-lg border border-line bg-surface px-3 py-2.5 text-sm">
      {body}
    </div>
  );
}

type RuleFieldsProps = {
  rule: Rule;
  timezone: string;
  onRule: (rule: Rule) => void;
  onTimezone: (timezone: string) => void;
  /** Messages by field (`cron`, `time`, `timezone`), from the form's check or the server. */
  errors: Record<string, string>;
};

/**
 * When a schedule runs: a preset (every hour, every day at, weekdays at, weekly on, monthly on
 * day) or a custom cron rule, the time zone (the device's by default), and the preview.
 */
export function RuleFields({ rule, timezone, onRule, onTimezone, errors }: RuleFieldsProps) {
  const zones = useMemo(() => zoneList(deviceZone(), timezone), [timezone]);
  const set = (change: Partial<Rule>) => onRule({ ...rule, ...change });
  const cron = Object.keys(ruleErrors(rule)).length === 0 ? toCron(rule) : "";

  return (
    <fieldset className="space-y-4">
      <legend className="field-label">When it runs</legend>
      <div className="grid gap-4 sm:grid-cols-2">
        <Select
          label="Repeat"
          value={rule.preset}
          onChange={(event) => {
            const preset = event.target.value as Preset;
            // Switching to custom starts from the rule the preset wrote.
            set(
              preset === "custom" && rule.preset !== "custom"
                ? { preset, cron: toCron(rule) }
                : { preset }
            );
          }}
        >
          {PRESETS.map((preset) => (
            <option key={preset.value} value={preset.value}>
              {preset.label}
            </option>
          ))}
        </Select>
        {rule.preset === "weekly" ? (
          <Select
            label="Day"
            value={rule.weekday}
            onChange={(event) => set({ weekday: Number(event.target.value) })}
          >
            {WEEKDAYS.map((day, index) => (
              <option key={day} value={index}>
                {day}
              </option>
            ))}
          </Select>
        ) : null}
        {rule.preset === "monthly" ? (
          <TextField
            label="Day of the month"
            type="number"
            inputMode="numeric"
            min={1}
            max={31}
            value={rule.day}
            error={errors.day}
            onChange={(event) => set({ day: Number(event.target.value) })}
            hint="Months without this day are skipped."
          />
        ) : null}
        {rule.preset === "hourly" ? (
          <TextField
            label="Minutes past the hour"
            type="number"
            inputMode="numeric"
            min={0}
            max={59}
            value={rule.minute}
            error={errors.time}
            onChange={(event) => set({ minute: Number(event.target.value) })}
          />
        ) : null}
        {rule.preset !== "hourly" && rule.preset !== "custom" ? (
          <TextField
            label="Time"
            type="time"
            value={rule.time}
            error={errors.time}
            onChange={(event) => set({ time: event.target.value })}
          />
        ) : null}
      </div>
      {rule.preset === "custom" ? (
        <TextField
          label="Cron rule"
          value={rule.cron}
          error={errors.cron}
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          placeholder="0 9 * * 1-5"
          className="field-input font-mono"
          onChange={(event) => set({ cron: event.target.value })}
          hint="Five fields: minute, hour, day of month, month, and day of week. Runs must be at least 15 minutes apart."
        />
      ) : errors.cron ? (
        <p className="field-error">{errors.cron}</p>
      ) : null}
      <Select
        label="Time zone"
        value={timezone}
        error={errors.timezone}
        onChange={(event) => onTimezone(event.target.value)}
      >
        {zones.map((zone) => (
          <option key={zone} value={zone}>
            {zone.replaceAll("_", " ")}
          </option>
        ))}
      </Select>
      <SchedulePreview cron={cron} timezone={timezone} />
    </fieldset>
  );
}
