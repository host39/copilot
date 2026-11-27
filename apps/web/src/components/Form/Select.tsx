import { type ReactNode, type SelectHTMLAttributes, useId } from "react";

type SelectProps = {
  label: string;
  hint?: ReactNode;
  /** A message for this field; marks the select invalid. */
  error?: string;
  children: ReactNode;
} & Omit<SelectHTMLAttributes<HTMLSelectElement>, "id">;

/** A labelled native `<select>` with its hint and error, wired up for screen readers. */
export function Select({ label, hint, error, children, ...rest }: SelectProps) {
  const id = useId();
  const described = [error ? `${id}-error` : "", hint ? `${id}-hint` : ""].join(" ").trim();
  return (
    <div>
      <label htmlFor={id} className="field-label">
        {label}
      </label>
      <select
        id={id}
        aria-invalid={error ? true : undefined}
        aria-describedby={described || undefined}
        className="field-input"
        {...rest}
      >
        {children}
      </select>
      {hint ? (
        <p id={`${id}-hint`} className="field-hint">
          {hint}
        </p>
      ) : null}
      {error ? (
        <p id={`${id}-error`} className="field-error">
          {error}
        </p>
      ) : null}
    </div>
  );
}
