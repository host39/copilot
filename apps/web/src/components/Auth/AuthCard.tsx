import { ServerLink } from "@web/components/Auth/ServerLink";
import { PageTitle } from "@web/components/Layout/PageTitle";
import type { ReactNode } from "react";

type AuthCardProps = {
  /** Heading and tab title, in title case. */
  title: string;
  description?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  /** Native builds show the server with a link to change it; the Connect screen turns it off. */
  server?: boolean;
};

/** The centered card of the sign-in, setup, and connect screens. */
export function AuthCard({ title, description, children, footer, server = true }: AuthCardProps) {
  return (
    <main className="flex min-h-dvh flex-col items-center justify-center bg-surface px-4 py-10">
      <PageTitle page={title} />
      <div className="w-full max-w-sm">
        <div className="mb-6 flex items-center justify-center gap-2">
          <img src="/icons/icon.svg" alt="" className="size-9" />
          <span className="text-lg font-semibold tracking-tight">Copilot</span>
        </div>
        <div className="card-panel">
          <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
          {description ? <div className="mt-2 text-sm text-muted">{description}</div> : null}
          <div className="mt-6">{children}</div>
        </div>
        {footer ? <div className="mt-6 text-center text-sm text-muted">{footer}</div> : null}
        {server ? <ServerLink className="mt-4 text-center text-sm text-muted" /> : null}
      </div>
    </main>
  );
}
