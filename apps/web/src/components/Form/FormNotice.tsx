import type { ReactNode } from "react";

/**
 * A success message ("Saved."). The live region is always in the page, so screen readers announce
 * each new message politely.
 */
export function FormNotice({ children }: { children?: ReactNode }) {
  return (
    <div role="status" aria-live="polite">
      {children ? (
        <p className="rounded-lg border border-ok/40 bg-ok/10 px-3 py-2.5 text-sm text-ok">
          {children}
        </p>
      ) : null}
    </div>
  );
}
