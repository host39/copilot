import { LuLoaderCircle } from "react-icons/lu";

/** A quiet spinner while a screen's code or data loads inside the app shell. */
export function PageLoading() {
  return (
    <div className="flex h-full min-h-40 items-center justify-center text-muted">
      <LuLoaderCircle className="size-6 animate-spin" aria-hidden />
      <span className="sr-only">Loading</span>
    </div>
  );
}
