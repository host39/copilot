import { type ReactNode, useId } from "react";

type SectionProps = {
  title: string;
  description?: ReactNode;
  /** Buttons beside the heading. */
  actions?: ReactNode;
  children: ReactNode;
};

/** A titled card on a Settings screen; the heading names the region for screen readers. */
export function Section({ title, description, actions, children }: SectionProps) {
  const id = useId();
  return (
    <section aria-labelledby={id} className="card-panel">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 id={id} className="text-base font-semibold">
            {title}
          </h2>
          {description ? <div className="mt-1 text-sm text-muted">{description}</div> : null}
        </div>
        {actions ? <div className="flex shrink-0 flex-wrap gap-2">{actions}</div> : null}
      </div>
      <div className="mt-4">{children}</div>
    </section>
  );
}
