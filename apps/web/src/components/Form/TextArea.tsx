import { type ReactNode, type TextareaHTMLAttributes, useId } from "react";

type TextAreaProps = {
  label: string;
  /** A message for this field; marks the input invalid. */
  error?: string;
  hint?: ReactNode;
} & Omit<TextareaHTMLAttributes<HTMLTextAreaElement>, "id">;

/** A labelled multi-line input with its hint and error, wired up for screen readers. */
export function TextArea({ label, error, hint, className, ...rest }: TextAreaProps) {
  const id = useId();
  const described = [error ? `${id}-error` : "", hint ? `${id}-hint` : ""].join(" ").trim();

  return (
    <div>
      <label htmlFor={id} className="field-label">
        {label}
      </label>
      <textarea
        id={id}
        aria-invalid={error ? true : undefined}
        aria-describedby={described || undefined}
        className={`field-input min-h-28 resize-y ${className ?? ""}`}
        {...rest}
      />
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
