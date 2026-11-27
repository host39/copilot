import { PageTitle } from "@web/components/Layout/PageTitle";
import type { ReactNode } from "react";
import { LuArrowLeft } from "react-icons/lu";
import { Link } from "react-router-dom";

type SettingsPageProps = {
  /** The tab title, in title case. */
  title: string;
  /** The page heading when it differs from `title` (a provider's name, for example). */
  heading?: ReactNode;
  description?: ReactNode;
  /** The list this page belongs to. */
  back?: { to: string; label: string };
  /** Buttons beside the heading. */
  actions?: ReactNode;
  children: ReactNode;
};

/** One Settings screen: a heading with its actions, then the screen's sections. */
export function SettingsPage({
  title,
  heading,
  description,
  back,
  actions,
  children,
}: SettingsPageProps) {
  return (
    <section className="px-4 py-6 md:px-0 md:py-0">
      <PageTitle page={title} />
      {back ? (
        <Link
          to={back.to}
          className="mb-3 inline-flex min-h-10 items-center gap-1.5 rounded-lg text-sm text-muted hover:text-fg focus-visible:outline-2 focus-visible:outline-accent"
        >
          <LuArrowLeft aria-hidden />
          {back.label}
        </Link>
      ) : null}
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h1 className="break-words text-xl font-semibold tracking-tight">{heading ?? title}</h1>
          {description ? <p className="mt-1 text-sm text-muted">{description}</p> : null}
        </div>
        {actions ? <div className="flex shrink-0 flex-wrap gap-2">{actions}</div> : null}
      </div>
      <div className="mt-6 space-y-6">{children}</div>
    </section>
  );
}
