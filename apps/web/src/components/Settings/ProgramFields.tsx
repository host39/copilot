import { TextArea } from "@web/components/Form/TextArea";
import { TextField } from "@web/components/Form/TextField";
import { HeaderRows } from "@web/components/Settings/HeaderRows";
import type { HeaderRow } from "@web/routes/settings/tool-server/forms";

type ProgramFieldsProps = {
  rows: HeaderRow[];
  onRows: (rows: HeaderRow[]) => void;
  /** Field errors by name (`command`, `args`, `env`, `cwd`). */
  errors: Partial<Record<string, string>>;
  /** Called on every change, so a preview can follow the fields. */
  onInput?: () => void;
  defaults?: { command: string; args: string; cwd: string };
  /** The hint of the variables when some are stored already. */
  envHint?: string;
};

/** A stdio program: its absolute path, its arguments, its environment, its directory. */
export function ProgramFields({
  rows,
  onRows,
  errors,
  onInput,
  defaults,
  envHint,
}: ProgramFieldsProps) {
  return (
    <div className="space-y-4" onInput={onInput}>
      <TextField
        label="Program"
        name="command"
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        placeholder="/usr/local/bin/my-mcp-server"
        defaultValue={defaults?.command}
        hint="Its absolute path on the Copilot server. It starts without PATH, so a bare name like npx is not looked up."
        error={errors.command}
      />
      <TextArea
        label="Arguments (optional)"
        name="args"
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        rows={3}
        placeholder={"--root\n/srv/data"}
        defaultValue={defaults?.args}
        hint="One argument per line, passed exactly as written. Nothing is expanded or split."
        error={errors.args}
      />
      <HeaderRows
        kind="variables"
        rows={rows}
        onChange={(next) => {
          onRows(next);
          onInput?.();
        }}
        error={errors.env}
        hint={envHint}
      />
      <TextField
        label="Working directory (optional)"
        name="cwd"
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        placeholder="github"
        defaultValue={defaults?.cwd}
        hint="A folder inside the server's stdio directory (MCP_STDIO_DIR), made when the program first starts. Empty: a folder of its own."
        error={errors.cwd}
      />
    </div>
  );
}
