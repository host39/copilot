import type { ApprovalMode, AuthMode, Transport } from "@copilot/contracts";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { keys } from "@web/api/routes/keys";
import { fetchCapabilities } from "@web/api/routes/tool-servers";
import Button from "@web/components/Button/Button";
import { ConfirmDialog } from "@web/components/Dialog/ConfirmDialog";
import { FormAlert } from "@web/components/Form/FormAlert";
import { TextField } from "@web/components/Form/TextField";
import { ApprovalPicker } from "@web/components/Settings/ApprovalPicker";
import { AuthPicker } from "@web/components/Settings/AuthPicker";
import { ClientFields } from "@web/components/Settings/ClientFields";
import { HeaderRows } from "@web/components/Settings/HeaderRows";
import { ProgramFields } from "@web/components/Settings/ProgramFields";
import { ProgramPreview } from "@web/components/Settings/ProgramPreview";
import { SettingsPage } from "@web/components/Settings/SettingsPage";
import { TransportPicker } from "@web/components/Settings/TransportPicker";
import { text } from "@web/forms";
import { useSubmit } from "@web/hooks/useSubmit";
import path, { link } from "@web/routes/path";
import {
  type AddedState,
  addServer,
  type HeaderRow,
  SLUG_MAX_CHARS,
  serverSchema,
} from "@web/routes/settings/tool-server/forms";
import { environment, parseArgs, programErrors } from "@web/routes/settings/tool-server/stdio";
import { useRef, useState } from "react";
import { useNavigate } from "react-router-dom";

/** What the form holds, as `serverSchema` reads it. */
type Values = Record<string, unknown> & {
  command: string;
  args: string;
  envRows: HeaderRow[];
  cwd: string;
};

/**
 * Settings, Tool servers, Add: registers the server, tests it, and opens its page. An
 * administrator on a server that allows it may add a stdio program instead of an address; the
 * whole command shows while it is entered and again, for a confirmation, before it is saved
 * (the test right after starts it).
 */
export function AddToolServer() {
  const client = useQueryClient();
  const navigate = useNavigate();
  const formRef = useRef<HTMLFormElement>(null);
  const [transport, setTransport] = useState<Transport>("http");
  const [approval, setApproval] = useState<ApprovalMode>("ask");
  const [auth, setAuth] = useState<AuthMode>("none");
  const [rows, setRows] = useState<HeaderRow[]>([]);
  const [envRows, setEnvRows] = useState<HeaderRow[]>([]);
  const [draft, setDraft] = useState({ command: "", args: "", cwd: "" });
  const [pending, setPending] = useState<Values>();
  const capabilities = useQuery({
    queryKey: keys.capabilities,
    queryFn: ({ signal }) => fetchCapabilities(signal),
  });
  const stdioAllowed = capabilities.data?.stdioServers === true;
  const stdio = stdioAllowed && transport === "stdio";
  const form = useSubmit(serverSchema, addServer, (added) => {
    setPending(undefined);
    void client.invalidateQueries({ queryKey: keys.toolServers });
    const state: AddedState = { test: added.test, problem: added.problem };
    navigate(link.toolServer(added.server.id), { replace: true, state });
  });

  const read = (element: HTMLFormElement): Values => {
    const data = new FormData(element);
    return {
      name: text(data, "name"),
      slug: text(data, "slug"),
      transport: stdio ? "stdio" : "http",
      url: text(data, "url"),
      approval,
      auth,
      bearer: text(data, "bearer"),
      rows,
      clientId: text(data, "clientId"),
      clientSecret: text(data, "clientSecret"),
      command: text(data, "command"),
      args: String(data.get("args") ?? ""),
      envRows,
      cwd: text(data, "cwd"),
    };
  };

  return (
    <SettingsPage
      title="Add Tool Server"
      back={{ to: path.toolServers, label: "Tool servers" }}
      description="A remote MCP server over Streamable HTTP. Copilot tests it by listing its tools; a server you sign in to is tested after the sign-in."
    >
      <form
        ref={formRef}
        noValidate
        className="card-panel space-y-6"
        onSubmit={(event) => {
          event.preventDefault();
          const values = read(event.currentTarget);
          if (stdio && serverSchema.safeParse(values).success) {
            setPending(values);
            return;
          }
          void form.submit(values);
        }}
      >
        <FormAlert>{form.alert}</FormAlert>
        <TextField
          label="Name"
          name="name"
          autoComplete="off"
          placeholder="GitHub"
          error={form.errors.name}
        />
        {stdioAllowed ? <TransportPicker value={transport} onChange={setTransport} /> : null}
        {stdio ? (
          <>
            <ProgramFields
              rows={envRows}
              onRows={setEnvRows}
              errors={programErrors(form.errors)}
              onInput={() => {
                const element = formRef.current;
                if (!element) return;
                const values = read(element);
                setDraft({ command: values.command, args: values.args, cwd: values.cwd });
              }}
            />
            <ProgramPreview
              command={draft.command.trim()}
              args={parseArgs(draft.args)}
              envNames={Object.keys(environment(envRows))}
              cwd={draft.cwd.trim()}
            />
          </>
        ) : (
          <>
            <TextField
              label="Server address"
              name="url"
              type="url"
              inputMode="url"
              autoComplete="off"
              autoCapitalize="none"
              spellCheck={false}
              placeholder="https://mcp.example.com/mcp"
              error={form.errors.url}
            />
            <AuthPicker value={auth} onChange={setAuth} />
            {auth === "headers" ? (
              <>
                <TextField
                  label="Bearer token"
                  name="bearer"
                  type="password"
                  autoComplete="off"
                  spellCheck={false}
                  hint="Sent as Authorization: Bearer. Stored encrypted; it is never shown again. Or add headers below."
                  error={form.errors.bearer}
                />
                <HeaderRows rows={rows} onChange={setRows} error={form.errors.headers} />
              </>
            ) : null}
            {auth === "oauth" ? <ClientFields error={form.errors.clientId} /> : null}
          </>
        )}
        <ApprovalPicker value={approval} onChange={setApproval} />
        <TextField
          label="Short name (optional)"
          name="slug"
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          maxLength={SLUG_MAX_CHARS}
          hint={`Prefixes this server's tool names for the model. Up to ${SLUG_MAX_CHARS} lowercase letters or digits; taken from the name when empty.`}
          error={form.errors.slug}
        />
        <div className="flex justify-end">
          <Button type="submit" busy={form.busy} disabled={form.wait > 0}>
            {form.busy ? "Connecting" : "Add and test"}
          </Button>
        </div>
      </form>
      <ConfirmDialog
        open={pending !== undefined}
        title="Run this program on the server?"
        confirm="Add and test"
        tone="primary"
        busy={form.busy}
        error={form.alert}
        onConfirm={() => {
          if (pending) void form.submit(pending);
        }}
        onCancel={() => setPending(undefined)}
      >
        {pending ? (
          <ProgramPreview
            title="Copilot saves this program and starts it to list its tools:"
            command={pending.command.trim()}
            args={parseArgs(pending.args)}
            envNames={Object.keys(environment(pending.envRows))}
            cwd={pending.cwd.trim()}
          />
        ) : null}
      </ConfirmDialog>
    </SettingsPage>
  );
}
