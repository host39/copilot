import Button from "@web/components/Button/Button";
import type { HeaderRow } from "@web/routes/settings/tool-server/forms";
import { useId, useState } from "react";
import { LuEye, LuEyeOff, LuPlus, LuTrash2 } from "react-icons/lu";

type HeaderRowsProps = {
  rows: HeaderRow[];
  onChange: (rows: HeaderRow[]) => void;
  /** Why the server or the form refused the headers. */
  error?: string;
};

let nextId = 1;

/** A new, empty header row. */
export function emptyRow(): HeaderRow {
  nextId += 1;
  return { id: nextId, name: "", value: "" };
}

/**
 * Custom request headers as name and value rows. Values are hidden like passwords (they usually
 * hold keys) until the user shows them.
 */
export function HeaderRows({ rows, onChange, error }: HeaderRowsProps) {
  const id = useId();
  const [revealed, setRevealed] = useState(false);
  const update = (row: HeaderRow, change: Partial<HeaderRow>) =>
    onChange(rows.map((item) => (item.id === row.id ? { ...item, ...change } : item)));

  return (
    <fieldset aria-describedby={error ? `${id}-error` : `${id}-hint`}>
      <legend className="field-label">Custom headers (optional)</legend>
      <p id={`${id}-hint`} className="field-hint">
        Sent with every request to this server, for example an API key header.
      </p>
      {rows.length > 0 ? (
        <ul className="mt-3 space-y-2">
          {rows.map((row, index) => (
            <li
              key={row.id}
              className="grid grid-cols-[1fr_auto] gap-2 sm:grid-cols-[1fr_1fr_auto]"
            >
              <input
                aria-label={`Header ${index + 1} name`}
                value={row.name}
                placeholder="X-Api-Key"
                autoComplete="off"
                autoCapitalize="none"
                spellCheck={false}
                onChange={(event) => update(row, { name: event.target.value })}
                aria-invalid={error ? true : undefined}
                className="field-input mt-0"
              />
              <Button
                variant="ghost"
                aria-label={`Remove header ${index + 1}`}
                onClick={() => onChange(rows.filter((item) => item.id !== row.id))}
                className="px-3 sm:order-last"
              >
                <LuTrash2 aria-hidden />
              </Button>
              <input
                aria-label={`Header ${index + 1} value`}
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
          Add header
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
