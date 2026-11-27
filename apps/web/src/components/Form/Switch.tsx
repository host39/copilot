import { type InputHTMLAttributes, type ReactNode, useId } from "react";

type SwitchProps = {
  label: ReactNode;
  /** Switches are controlled, so `aria-checked` always matches what the user sees. */
  checked: boolean;
  hint?: ReactNode;
  /** Shows the label to screen readers only (a toggle in a list row whose name is beside it). */
  hideLabel?: boolean;
} & Omit<
  InputHTMLAttributes<HTMLInputElement>,
  "id" | "type" | "role" | "checked" | "defaultChecked"
>;

/** An on/off toggle: a checkbox with the `switch` role, drawn as a track and a knob. */
export function Switch({
  label,
  checked,
  hint,
  hideLabel = false,
  className,
  ...rest
}: SwitchProps) {
  const id = useId();
  return (
    <div className={`flex items-center justify-between gap-4 ${className ?? ""}`}>
      <div className={hideLabel ? "sr-only" : "min-w-0"}>
        <label htmlFor={id} className="field-label">
          {label}
        </label>
        {hint ? (
          <p id={`${id}-hint`} className="text-sm text-muted">
            {hint}
          </p>
        ) : null}
      </div>
      <span className="relative inline-flex shrink-0">
        <input
          id={id}
          type="checkbox"
          role="switch"
          checked={checked}
          aria-checked={checked}
          aria-describedby={hint ? `${id}-hint` : undefined}
          className="peer absolute inset-0 z-10 m-0 size-full cursor-pointer opacity-0 disabled:cursor-not-allowed"
          {...rest}
        />
        <span
          aria-hidden
          className="h-6 w-11 rounded-full bg-line transition-colors peer-checked:bg-accent peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-accent peer-disabled:opacity-60 after:absolute after:top-0.5 after:left-0.5 after:size-5 after:rounded-full after:bg-canvas after:shadow-sm after:transition-transform peer-checked:after:translate-x-5"
        />
      </span>
    </div>
  );
}
