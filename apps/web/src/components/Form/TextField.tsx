import { type InputHTMLAttributes, type ReactNode, useId, useState } from "react";
import { LuEye, LuEyeOff } from "react-icons/lu";

type TextFieldProps = {
  label: string;
  /** A message for this field; marks the input invalid. */
  error?: string;
  hint?: ReactNode;
} & Omit<InputHTMLAttributes<HTMLInputElement>, "id">;

/** A labelled input with its hint and error, wired up for screen readers. */
export function TextField({ label, error, hint, type = "text", ...rest }: TextFieldProps) {
  const id = useId();
  const [revealed, setRevealed] = useState(false);
  const password = type === "password";
  const described = [error ? `${id}-error` : "", hint ? `${id}-hint` : ""].join(" ").trim();

  return (
    <div>
      <label htmlFor={id} className="field-label">
        {label}
      </label>
      <div className="relative">
        <input
          id={id}
          type={password && revealed ? "text" : type}
          aria-invalid={error ? true : undefined}
          aria-describedby={described || undefined}
          className={`field-input ${password ? "pr-11" : ""}`}
          {...rest}
        />
        {password ? (
          <button
            type="button"
            onClick={() => setRevealed((value) => !value)}
            aria-label={revealed ? "Hide password" : "Show password"}
            aria-pressed={revealed}
            className="absolute inset-y-0 right-0 flex w-11 items-center justify-center rounded-r-lg text-muted hover:text-fg"
          >
            {revealed ? <LuEyeOff aria-hidden /> : <LuEye aria-hidden />}
          </button>
        ) : null}
      </div>
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
