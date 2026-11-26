import type { ReactNode } from "react";

/** A form-level message (a failed request), announced to screen readers when it appears. */
export function FormAlert({ children }: { children?: ReactNode }) {
  if (!children) return null;
  return (
    <p role="alert" className="form-alert">
      {children}
    </p>
  );
}
