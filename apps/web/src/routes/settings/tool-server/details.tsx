import type { ApprovalMode, ServerTool, ToolMode, ToolServerResponse } from "@copilot/contracts";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { keys } from "@web/api/routes/keys";
import {
  deleteToolServer,
  fetchToolServer,
  reviewToolServer,
  testToolServer,
  updateToolServer,
} from "@web/api/routes/tool-servers";
import { Badge } from "@web/components/Badge/Badge";
import Button from "@web/components/Button/Button";
import { ConfirmDialog } from "@web/components/Dialog/ConfirmDialog";
import { FormAlert } from "@web/components/Form/FormAlert";
import { FormNotice } from "@web/components/Form/FormNotice";
import { Switch } from "@web/components/Form/Switch";
import { TextField } from "@web/components/Form/TextField";
import { ApprovalPicker } from "@web/components/Settings/ApprovalPicker";
import { HeaderRows } from "@web/components/Settings/HeaderRows";
import { QueryState } from "@web/components/Settings/QueryState";
import { Section } from "@web/components/Settings/Section";
import { SettingsPage } from "@web/components/Settings/SettingsPage";
import { text } from "@web/forms";
import { useSubmit } from "@web/hooks/useSubmit";
import path from "@web/routes/path";
import { ago } from "@web/routes/settings/format";
import {
  type AddedState,
  credentialsSchema,
  type HeaderRow,
  settingsSchema,
} from "@web/routes/settings/tool-server/forms";
import {
  modeFor,
  modesBody,
  modesChanged,
  sortTools,
  testSummary,
} from "@web/routes/settings/tool-server/modes";
import { useState } from "react";
import { LuPlugZap } from "react-icons/lu";
import { useLocation, useNavigate } from "react-router-dom";

const BACK = { to: path.toolServers, label: "Tool servers" };

const MODES: { value: ToolMode; label: string }[] = [
  { value: "off", label: "Off" },
  { value: "ask", label: "Ask" },
  { value: "auto", label: "Auto" },
];

/** Shows a saved server at once, then refreshes the list. */
function useStore() {
  const client = useQueryClient();
  return (saved: ToolServerResponse) => {
    client.setQueryData(keys.toolServer(saved.id), saved);
    void client.invalidateQueries({ queryKey: keys.toolServers });
  };
}

function ModePicker({
  tool,
  value,
  onChange,
}: {
  tool: ServerTool;
  value: ToolMode;
  onChange: (mode: ToolMode) => void;
}) {
  return (
    <fieldset className="shrink-0">
      <legend className="sr-only">{`Mode for ${tool.title ?? tool.name}`}</legend>
      <div className="inline-flex rounded-lg border border-line p-0.5">
        {MODES.map((mode) => (
          <label
            key={mode.value}
            className="cursor-pointer rounded-md px-3 py-1.5 text-sm text-muted has-checked:bg-accent has-checked:text-on-accent has-focus-visible:outline-2 has-focus-visible:outline-accent"
          >
            <input
              type="radio"
              name={`mode-${tool.name}`}
              value={mode.value}
              checked={value === mode.value}
              onChange={() => onChange(mode.value)}
              className="sr-only"
            />
            {mode.label}
          </label>
        ))}
      </div>
    </fieldset>
  );
}

/** One listed tool: its title, name, description, read-only hint, and mode. */
function ToolRow({
  tool,
  mode,
  onChange,
}: {
  tool: ServerTool;
  mode: ToolMode;
  onChange: (mode: ToolMode) => void;
}) {
  return (
    <li className="flex flex-col gap-3 py-3 sm:flex-row sm:items-start">
      <div className="min-w-0 flex-1">
        <p className="flex flex-wrap items-center gap-2 text-sm font-medium">
          <span className="break-words">{tool.title ?? tool.name}</span>
          {tool.readOnly ? <Badge>Read-only</Badge> : null}
        </p>
        {tool.title ? <p className="break-all font-mono text-xs text-muted">{tool.name}</p> : null}
        {tool.description ? (
          <p className="mt-1 line-clamp-3 text-sm text-muted">{tool.description}</p>
        ) : null}
        {tool.readOnly && mode === "ask" ? (
          <p className="mt-1 text-xs text-muted">
            The server says this tool only reads, so Auto may suit it. Servers can be wrong about
            this.
          </p>
        ) : null}
      </div>
      <ModePicker tool={tool} value={mode} onChange={onChange} />
    </li>
  );
}

function Tools({ server, state }: { server: ToolServerResponse; state: AddedState | null }) {
  const store = useStore();
  const [draft, setDraft] = useState<Record<string, ToolMode>>({});
  const [notice, setNotice] = useState(
    state?.test ? testSummary(state.test.server.tools?.length ?? 0, state.test.changes) : undefined
  );
  const test = useMutation({
    mutationFn: () => testToolServer(server.id),
    onMutate: () => setNotice(undefined),
    onSuccess: (result) => {
      store(result.server);
      setNotice(testSummary(result.server.tools?.length ?? 0, result.changes));
    },
  });
  const save = useMutation({
    mutationFn: () => updateToolServer(server.id, { toolModes: modesBody(server, draft) }),
    onSuccess: (saved) => {
      store(saved);
      setDraft({});
      setNotice("Tool modes saved.");
    },
  });
  const review = useMutation({
    mutationFn: () => reviewToolServer(server.id),
    onSuccess: (saved) => {
      store(saved);
      setNotice("Changes accepted. Tools follow their modes again.");
    },
  });
  const tools = sortTools(server.tools ?? []);
  const dirty = modesChanged(server, draft);
  const failure = test.error ?? save.error ?? review.error;
  const problem = failure
    ? describeError(failure).message
    : test.isIdle && state?.problem
      ? `The connection test failed: ${state.problem}`
      : undefined;

  return (
    <Section
      title="Tools"
      description={
        server.toolsFetchedAt
          ? `Listed ${ago(server.toolsFetchedAt)}. Off tools are never offered to the model.`
          : "Test the server to list its tools."
      }
      actions={
        <Button variant="secondary" busy={test.isPending} onClick={() => test.mutate()}>
          {test.isPending ? null : <LuPlugZap aria-hidden />}
          Test connection
        </Button>
      }
    >
      <div className="space-y-4">
        <FormAlert>{problem}</FormAlert>
        <FormNotice>{notice}</FormNotice>
        {server.reviewRequired ? (
          <div className="form-warning flex flex-wrap items-center justify-between gap-3">
            <p>
              The server changed its tools since you last reviewed them. Every tool asks before it
              runs until you accept the changes.
            </p>
            <Button variant="secondary" busy={review.isPending} onClick={() => review.mutate()}>
              Accept changes
            </Button>
          </div>
        ) : null}
        {server.tools === null ? (
          <p className="text-sm text-muted">No tools listed yet.</p>
        ) : tools.length === 0 ? (
          <p className="text-sm text-muted">The server lists no tools.</p>
        ) : (
          <ul className="divide-y divide-line">
            {tools.map((tool) => (
              <ToolRow
                key={tool.name}
                tool={tool}
                mode={draft[tool.name] ?? modeFor(server, tool.name)}
                onChange={(next) => {
                  setNotice(undefined);
                  setDraft((current) => ({ ...current, [tool.name]: next }));
                }}
              />
            ))}
          </ul>
        )}
        {dirty ? (
          <div className="sticky bottom-0 flex flex-wrap items-center justify-end gap-2 border-t border-line bg-canvas py-3">
            <span className="mr-auto text-sm text-muted">Unsaved tool modes</span>
            <Button variant="secondary" onClick={() => setDraft({})} disabled={save.isPending}>
              Discard
            </Button>
            <Button busy={save.isPending} onClick={() => save.mutate()}>
              Save tool modes
            </Button>
          </div>
        ) : null}
      </div>
    </Section>
  );
}

function Settings({ server }: { server: ToolServerResponse }) {
  const store = useStore();
  const [approval, setApproval] = useState<ApprovalMode>(server.approval);
  const [notice, setNotice] = useState<string>();
  const form = useSubmit(
    settingsSchema(server),
    async (body) => (Object.keys(body).length === 0 ? server : updateToolServer(server.id, body)),
    (saved) => {
      store(saved);
      setNotice(
        saved.url !== server.url
          ? "Saved. Test the server to list the tools at its new address."
          : "Saved."
      );
    }
  );
  const enable = useMutation({
    mutationFn: (enabled: boolean) => updateToolServer(server.id, { enabled }),
    onSuccess: store,
  });
  const enabled = enable.isPending ? (enable.variables ?? server.enabled) : server.enabled;

  return (
    <Section title="Server" description={`Tool names reach the model as ${server.slug}_<tool>.`}>
      <div className="space-y-6">
        <Switch
          label="Use this server"
          hint="When it is off, no chat or project gets its tools."
          checked={enabled}
          disabled={enable.isPending}
          onChange={(event) => enable.mutate(event.target.checked)}
        />
        <FormAlert>{enable.error ? describeError(enable.error).message : null}</FormAlert>
        <form
          key={server.updatedAt}
          noValidate
          className="space-y-4"
          onSubmit={(event) => {
            event.preventDefault();
            setNotice(undefined);
            const data = new FormData(event.currentTarget);
            void form.submit({ name: text(data, "name"), url: text(data, "url"), approval });
          }}
        >
          <FormAlert>{form.alert}</FormAlert>
          <FormNotice>{notice}</FormNotice>
          <TextField
            label="Name"
            name="name"
            autoComplete="off"
            defaultValue={server.name}
            error={form.errors.name}
          />
          <TextField
            label="Server address"
            name="url"
            type="url"
            inputMode="url"
            autoComplete="off"
            autoCapitalize="none"
            spellCheck={false}
            defaultValue={server.url}
            hint="A new address asks before every tool until you accept its tools. Moving to another host removes the stored credentials unless you enter new ones."
            error={form.errors.url}
          />
          <ApprovalPicker value={approval} onChange={setApproval} />
          <div className="flex justify-end">
            <Button type="submit" busy={form.busy} disabled={form.wait > 0}>
              Save server
            </Button>
          </div>
        </form>
      </div>
    </Section>
  );
}

function CredentialsBadge({ server }: { server: ToolServerResponse }) {
  switch (server.credentials) {
    case "set":
      return <Badge tone="ok">Stored</Badge>;
    case "unreadable":
      return <Badge tone="danger">Unreadable: enter them again</Badge>;
    default:
      return <Badge>None</Badge>;
  }
}

function Credentials({ server }: { server: ToolServerResponse }) {
  const store = useStore();
  const [rows, setRows] = useState<HeaderRow[]>([]);
  const [notice, setNotice] = useState<string>();
  const [removing, setRemoving] = useState(false);
  const [round, setRound] = useState(0);
  const form = useSubmit(
    credentialsSchema,
    (credentials) => updateToolServer(server.id, { credentials }),
    (saved) => {
      store(saved);
      setRows([]);
      setRound((value) => value + 1);
      setNotice("Credentials replaced.");
    }
  );
  const remove = useMutation({
    mutationFn: () => updateToolServer(server.id, { credentials: null }),
    onSuccess: (saved) => {
      store(saved);
      setRemoving(false);
      setNotice("Credentials removed.");
    },
  });

  return (
    <Section
      title="Credentials"
      description={
        <span className="flex flex-wrap items-center gap-2">
          A bearer token and custom headers, stored encrypted and never shown again.
          <CredentialsBadge server={server} />
        </span>
      }
    >
      <form
        key={round}
        noValidate
        className="space-y-4"
        onSubmit={(event) => {
          event.preventDefault();
          setNotice(undefined);
          void form.submit({ bearer: text(new FormData(event.currentTarget), "bearer"), rows });
        }}
      >
        <FormAlert>{form.alert}</FormAlert>
        <FormNotice>{notice}</FormNotice>
        <TextField
          label="Bearer token"
          name="bearer"
          type="password"
          autoComplete="off"
          spellCheck={false}
          hint="Saving replaces the stored token and headers together."
          error={form.errors.bearer}
        />
        <HeaderRows rows={rows} onChange={setRows} error={form.errors.headers} />
        <div className="flex flex-wrap justify-end gap-2">
          {server.credentials === "missing" ? null : (
            <Button variant="secondary" onClick={() => setRemoving(true)}>
              Remove credentials
            </Button>
          )}
          <Button type="submit" busy={form.busy} disabled={form.wait > 0}>
            {server.credentials === "missing" ? "Save credentials" : "Replace credentials"}
          </Button>
        </div>
      </form>
      <ConfirmDialog
        open={removing}
        title="Remove the credentials?"
        confirm="Remove credentials"
        busy={remove.isPending}
        error={remove.error ? describeError(remove.error).message : undefined}
        onConfirm={() => remove.mutate()}
        onCancel={() => {
          setRemoving(false);
          remove.reset();
        }}
      >
        <p>The bearer token and every custom header are deleted.</p>
      </ConfirmDialog>
    </Section>
  );
}

function Remove({ server }: { server: ToolServerResponse }) {
  const client = useQueryClient();
  const navigate = useNavigate();
  const [open, setOpen] = useState(false);
  const remove = useMutation({
    mutationFn: () => deleteToolServer(server.id),
    onSuccess: () => {
      client.removeQueries({ queryKey: keys.toolServer(server.id) });
      void client.invalidateQueries({ queryKey: keys.toolServers });
      navigate(path.toolServers, { replace: true });
    },
  });

  return (
    <Section
      title="Delete Tool Server"
      description="Removes the server from every project and chat."
    >
      <Button variant="danger" onClick={() => setOpen(true)}>
        Delete tool server
      </Button>
      <ConfirmDialog
        open={open}
        title={`Delete ${server.name}?`}
        confirm="Delete tool server"
        busy={remove.isPending}
        error={remove.error ? describeError(remove.error).message : undefined}
        onConfirm={() => remove.mutate()}
        onCancel={() => {
          setOpen(false);
          remove.reset();
        }}
      >
        <p>
          Projects and chats that use it lose its tools, and the approvals given for it in chats are
          deleted. This cannot be undone.
        </p>
      </ConfirmDialog>
    </Section>
  );
}

/** Settings, Tool servers, one server: its tools and their modes, settings, credentials, delete. */
export function ToolServerDetails({ id }: { id: string }) {
  const state = useLocation().state as AddedState | null;
  const server = useQuery({
    queryKey: keys.toolServer(id),
    queryFn: ({ signal }) => fetchToolServer(id, signal),
  });
  const data = server.data;

  return (
    <SettingsPage title="Tool Server" heading={data?.name} description={data?.url} back={BACK}>
      <QueryState query={server} back={BACK}>
        {data ? (
          <>
            <Tools server={data} state={state} />
            <Settings server={data} />
            <Credentials server={data} />
            <Remove server={data} />
          </>
        ) : null}
      </QueryState>
    </SettingsPage>
  );
}
