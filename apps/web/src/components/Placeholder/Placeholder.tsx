import { PageTitle } from "@web/components/Layout/PageTitle";
import type { ReactNode } from "react";

type PlaceholderProps = {
  /** The screen's name in title case; also the tab title. */
  title: string;
  children?: ReactNode;
};

/** Stands in for a screen that a later step builds. */
export function Placeholder({ title, children }: PlaceholderProps) {
  return (
    <section className="mx-auto w-full max-w-3xl px-4 py-8 sm:px-8">
      <PageTitle page={title} />
      <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
      <p className="mt-2 text-sm text-muted">This screen is not built yet.</p>
      {children ? <div className="mt-6">{children}</div> : null}
    </section>
  );
}
