import { ApiError } from "@web/api/client";
import { describeError } from "@web/api/errors";
import Button from "@web/components/Button/Button";
import { PageLoading } from "@web/components/Suspense/PageLoading";
import type { ReactNode } from "react";
import { Link } from "react-router-dom";

type QueryStateProps = {
  /** The query's state, from TanStack Query. */
  query: { isPending: boolean; isError: boolean; error: unknown; refetch: () => unknown };
  /** Where "not found" sends the user back to. */
  back?: { to: string; label: string };
  children: ReactNode;
};

/**
 * What a Settings screen shows while its data loads or after it failed: a spinner, a "not found"
 * message for a `404` (another user's id, or something deleted meanwhile), or the server's error
 * with a retry button. The children render once the data is there.
 */
export function QueryState({ query, back, children }: QueryStateProps) {
  if (query.isPending) return <PageLoading />;
  if (query.isError) {
    const missing = query.error instanceof ApiError && query.error.status === 404;
    return (
      <div role="alert" className="card-panel space-y-4">
        <p className="text-sm">
          {missing ? "This item does not exist anymore." : describeError(query.error).message}
        </p>
        <div className="flex flex-wrap gap-2">
          {missing ? null : (
            <Button variant="secondary" onClick={() => void query.refetch()}>
              Try again
            </Button>
          )}
          {back ? (
            <Link to={back.to} className="text-link inline-flex min-h-10 items-center">
              {back.label}
            </Link>
          ) : null}
        </div>
      </div>
    );
  }
  return <>{children}</>;
}
