import Button from "@web/components/Button/Button";
import type { HeaderRow } from "@web/routes/settings/tool-server/forms";
import { useId, useState } from "react";
import { LuEye, LuEyeOff, LuPlus, LuTrash2 } from "react-icons/lu";

type HeaderRowsProps = {
  rows: HeaderRow[];
  onChange: (rows: HeaderRow[]) => void;
  /** Why the server or the form refused the headers. */
  error?: string;
  /** `variables`: a stdio program's environment variables instead of request headers. */
  kind?: "headers" | "variables";
  /** Replaces the default hint. */
  hint?: string;
};

const TEXT = {
  headers: {
    legend: "Custom headers (optional)",
    hint: "Sent with every request to this server, for example an API key header.",
    noun: "Header",
    add: "Add header",
    placeholder: "X-Api-Key",
  },
  variables: {
    legend: "Environment variables (optional)",
    hint: "The program's whole environment: it gets these and nothing else, so set what it needs, such as an API key. Stored encrypted; values are never shown again.",
    noun: "Variable",
    add: "Add variable",
    placeholder: "API_KEY",
  },
} as const;

let nextId = 1;

/** A new, empty header row. */
export function emptyRow(): HeaderRow {
  nextId += 1;
  return { id: nextId, name: "", value: "" };
}

/**
 * Custom request headers (or a program's environment variables) as name and value rows. Values
 * are hidden like passwords (they usually hold keys) until the user shows them.
 */
export function HeaderRows({ rows, onChange, error, kind = "headers", hint }: HeaderRowsProps) {
  const text = TEXT[kind];
  const id = useId();
  const [revealed, setRevealed] = useState(false);
  const update = (row: HeaderRow, change: Partial<HeaderRow>) =>
    onChange(rows.map((item) => (item.id === row.id ? { ...item, ...change } : item)));

  return (
    <fieldset aria-describedby={error ? `${id}-error` : `${id}-hint`}>
      <legend className="field-label">{text.legend}</legend>
      <p id={`${id}-hint`} className="field-hint">
        {hint ?? text.hint}
      </p>
      {rows.length > 0 ? (
        <ul className="mt-3 space-y-2">
          {rows.map((row, index) => (
            <li
              key={row.id}
              className="grid grid-cols-[1fr_auto] gap-2 sm:grid-cols-[1fr_1fr_auto]"
            >
              <input
                aria-label={`${text.noun} ${index + 1} name`}
                value={row.name}
                placeholder={text.placeholder}
                autoComplete="off"
                autoCapitalize="none"
                spellCheck={false}
                onChange={(event) => update(row, { name: event.target.value })}
                aria-invalid={error ? true : undefined}
                className="field-input mt-0"
              />
              <Button
                variant="ghost"
                aria-label={`Remove ${text.noun.toLowerCase()} ${index + 1}`}
                onClick={() => onChange(rows.filter((item) => item.id !== row.id))}
                className="px-3 sm:order-last"
              >
                <LuTrash2 aria-hidden />
              </Button>
              <input
                aria-label={`${text.noun} ${index + 1} value`}
                type={revealed ? "text" : "password"}
                value={row.value}
                placeholder="Value"
                autoComplete="off"
                spellCheck={false}
                onChange={(event) => update(row, { value: event.target.value })}
                aria-invalid={error ? true : undefined}
                className="field-input col-span-2 mt-0 sm:col-span-1"
              />
            </li>
          ))}
        </ul>
      ) : null}
      {error ? (
        <p id={`${id}-error`} className="field-error">
          {error}
        </p>
      ) : null}
      <div className="mt-2 flex flex-wrap gap-2">
        <Button variant="secondary" onClick={() => onChange([...rows, emptyRow()])}>
          <LuPlus aria-hidden />
          {text.add}
        </Button>
        {rows.length > 0 ? (
          <Button
            variant="ghost"
            aria-pressed={revealed}
            onClick={() => setRevealed((value) => !value)}
          >
            {revealed ? <LuEyeOff aria-hidden /> : <LuEye aria-hidden />}
            {revealed ? "Hide values" : "Show values"}
          </Button>
        ) : null}
      </div>
    </fieldset>
  );
}
