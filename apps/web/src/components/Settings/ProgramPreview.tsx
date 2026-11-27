import { commandLine } from "@web/routes/settings/tool-server/stdio";

type ProgramPreviewProps = {
  command: string;
  args: readonly string[];
  /** The environment variable names (their values never show). */
  envNames: readonly string[];
  /** The working directory under the server's stdio folder; blank: the server's own. */
  cwd: string;
  title?: string;
};

/**
 * A stdio program exactly as it will run: the command and every argument, wrapped but never
 * shortened, with its environment variable names and working directory.
 */
export function ProgramPreview({
  command,
  args,
  envNames,
  cwd,
  title = "The program Copilot runs",
}: ProgramPreviewProps) {
  return (
    <div className="space-y-2">
      <p className="field-label">{title}</p>
      <pre className="program-line">{commandLine(command, args)}</pre>
      <ul className="space-y-1 text-sm text-muted">
        <li>
          Run directly, without a shell, as the Copilot server's own user. Quotes only show where an
          argument starts and ends.
        </li>
        <li>
          {envNames.length === 0
            ? "Environment: empty (none of the server's own variables)."
            : `Environment: only ${envNames.join(", ")}.`}
        </li>
        <li>
          {cwd === ""
            ? "Working directory: its own folder under the server's stdio directory."
            : `Working directory: ${cwd} under the server's stdio directory.`}
        </li>
      </ul>
    </div>
  );
}
