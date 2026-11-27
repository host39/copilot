import { isNative, serverUrl } from "@web/config";
import path from "@web/routes/path";
import { Link } from "react-router-dom";

/**
 * Native builds: the server the app signs in to, with a link to the Connect screen to change it.
 * Renders nothing on the web, where the page's own server is the only one.
 */
export function ServerLink({ className }: { className?: string }) {
  const url = isNative() ? serverUrl() : undefined;
  if (url === undefined) return null;
  return (
    <p className={className}>
      Server <span className="break-all text-fg">{url}</span>{" "}
      <Link to={path.connect} className="text-link">
        Change
      </Link>
    </p>
  );
}
