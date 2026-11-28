import type {
  ApprovalMode,
  AuthMode,
  ServerTool,
  StdioProgram,
  ToolMode,
  ToolServerResponse,
} from "@copilot/contracts";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { keys } from "@web/api/routes/keys";
import {
  confirmSignIn,
  deleteToolServer,
  fetchCapabilities,
  fetchToolServer,
  reviewToolServer,
  signOut,
  startSignIn,
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
import { AuthPicker } from "@web/components/Settings/AuthPicker";
import { ClientFields } from "@web/components/Settings/ClientFields";
import { HeaderRows } from "@web/components/Settings/HeaderRows";
import { ProgramFields } from "@web/components/Settings/ProgramFields";
import { ProgramPreview } from "@web/components/Settings/ProgramPreview";
import { QueryState } from "@web/components/Settings/QueryState";
import { Section } from "@web/components/Settings/Section";
import { SettingsPage } from "@web/components/Settings/SettingsPage";
import { isNative } from "@web/config";
import { text } from "@web/forms";
import { useSubmit } from "@web/hooks/useSubmit";
import path from "@web/routes/path";
import { ago } from "@web/routes/settings/format";
import {
  type AddedState,
  clientSchema,
  confirmSchema,
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
import {
  openSignIn,
  SIGN_IN_POLL_MS,
  SIGN_IN_WAIT_MS,
  signInBadge,
  signInMark,
} from "@web/routes/settings/tool-server/sign-in";
import {
  envNamesAfter,
  location,
  type ProgramInput,
  programErrors,
  programSchema,
} from "@web/routes/settings/tool-server/stdio";
import { useEffect, useState } from "react";
import { LuLogIn, LuPlugZap } from "react-icons/lu";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";

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

/** Changed tools wait for the user's review; a stdio server shows the program it reviews. */
function Review({
  server,
  busy,
  onAccept,
}: {
  server: ToolServerResponse;
  busy: boolean;
  onAccept: () => void;
}) {
  return (
    <div className="form-warning flex flex-wrap items-center justify-between gap-3">
      <p>
        The server changed its tools since you last reviewed them. Every tool asks before it runs
        until you accept the changes.
      </p>
      <Button variant="secondary" busy={busy} onClick={onAccept}>
        Accept changes
      </Button>
      {server.stdio ? (
        <div className="w-full text-fg">
          <ProgramPreview
            title="You accept the tools of this program:"
            command={server.stdio.command}
            args={server.stdio.args}
            envNames={server.stdio.envNames}
            cwd={server.stdio.cwd}
          />
        </div>
      ) : null}
    </div>
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
          <Review server={server} busy={review.isPending} onAccept={() => review.mutate()} />
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
            const url = server.transport === "stdio" ? undefined : text(data, "url");
            void form.submit({ name: text(data, "name"), url, approval });
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
          {server.url === null ? null : (
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
          )}
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

/**
 * A stdio server's program, always shown whole. An administrator (while the server allows stdio)
 * may change it ([`ProgramForm`]).
 */
function Program({ server }: { server: ToolServerResponse }) {
  const capabilities = useQuery({
    queryKey: keys.capabilities,
    queryFn: ({ signal }) => fetchCapabilities(signal),
  });
  const program = server.stdio;
  if (!program) return null;

  return (
    <Section
      title="Program"
      description="Copilot starts this program for each chat that uses the server, and for each test, and ends it afterwards."
    >
      <div className="space-y-6">
        <ProgramPreview
          command={program.command}
          args={program.args}
          envNames={program.envNames}
          cwd={program.cwd}
        />
        {program.env === "unreadable" ? (
          <p className="form-warning">
            The stored environment variables can no longer be read: enter them again.
          </p>
        ) : null}
        {capabilities.data?.stdioServers ? (
          <ProgramForm key={server.updatedAt} server={server} program={program} />
        ) : (
          <p className="text-sm text-muted">
            Only an administrator can change the program, while this Copilot server allows stdio
            tool servers (MCP_STDIO_ENABLED).
          </p>
        )}
      </div>
    </Section>
  );
}

/** The directory field's value: blank for the server's own folder. */
function ownCwd(server: ToolServerResponse, program: StdioProgram): string {
  return program.cwd === server.id ? "" : program.cwd;
}

/** Changes the program; the new one shows again for a confirmation before it is saved. */
function ProgramForm({ server, program }: { server: ToolServerResponse; program: StdioProgram }) {
  const store = useStore();
  const [rows, setRows] = useState<HeaderRow[]>([]);
  const [clearEnv, setClearEnv] = useState(false);
  const [notice, setNotice] = useState<string>();
  const [pending, setPending] = useState<ProgramInput>();
  const schema = programSchema(server, clearEnv);
  const form = useSubmit(
    schema,
    async (stdio) =>
      Object.keys(stdio).length === 0 ? server : updateToolServer(server.id, { stdio }),
    (saved) => {
      setPending(undefined);
      store(saved);
      setNotice(
        saved.updatedAt === server.updatedAt
          ? "Nothing changed."
          : "Saved. Test the server to list the tools of the new program."
      );
    }
  );
  const parsed = pending ? schema.safeParse(pending) : undefined;
  const next = parsed?.success ? parsed.data : undefined;

  return (
    <form
      noValidate
      className="space-y-4 border-t border-line pt-4"
      onSubmit={(event) => {
        event.preventDefault();
        setNotice(undefined);
        const data = new FormData(event.currentTarget);
        const values: ProgramInput = {
          command: text(data, "command"),
          args: String(data.get("args") ?? ""),
          rows,
          cwd: text(data, "cwd"),
        };
        const checked = schema.safeParse(values);
        if (checked.success && Object.keys(checked.data).length > 0) setPending(values);
        else void form.submit(values);
      }}
    >
      <FormAlert>{form.alert}</FormAlert>
      <FormNotice>{notice}</FormNotice>
      <ProgramFields
        rows={rows}
        onRows={setRows}
        errors={programErrors(form.errors)}
        defaults={{
          command: program.command,
          args: program.args.join("\n"),
          cwd: ownCwd(server, program),
        }}
        envHint={
          program.envNames.length > 0
            ? `Stored: ${program.envNames.join(", ")}. Variables entered here replace them all; leave this empty to keep them.`
            : undefined
        }
      />
      {program.envNames.length > 0 ? (
        <Switch
          label="Remove the stored variables"
          hint="When no variables are entered above, the program starts with an empty environment."
          checked={clearEnv}
          onChange={(event) => setClearEnv(event.target.checked)}
        />
      ) : null}
      <div className="flex justify-end">
        <Button type="submit" busy={form.busy} disabled={form.wait > 0}>
          Save program
        </Button>
      </div>
      <ConfirmDialog
        open={pending !== undefined}
        title="Save this program?"
        confirm="Save program"
        tone="primary"
        busy={form.busy}
        error={form.alert}
        onConfirm={() => {
          if (pending) void form.submit(pending);
        }}
        onCancel={() => setPending(undefined)}
      >
        {next ? (
          <ProgramPreview
            title="Copilot runs this program from the next chat or test on:"
            command={next.command ?? program.command}
            args={next.args ?? program.args}
            envNames={envNamesAfter(server, next)}
            cwd={next.cwd ?? ownCwd(server, program)}
          />
        ) : null}
      </ConfirmDialog>
    </form>
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

/** The bearer token and custom headers. Saving them switches the server to them. */
function HeaderCredentials({ server }: { server: ToolServerResponse }) {
  const store = useStore();
  const [rows, setRows] = useState<HeaderRow[]>([]);
  const [notice, setNotice] = useState<string>();
  const [removing, setRemoving] = useState(false);
  const [round, setRound] = useState(0);
  const current = server.authMode === "headers";
  const form = useSubmit(
    credentialsSchema,
    (credentials) => updateToolServer(server.id, { credentials }),
    (saved) => {
      store(saved);
      setRows([]);
      setRound((value) => value + 1);
      setNotice(current ? "Credentials replaced." : "Credentials saved.");
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
    <>
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
        <p className="flex flex-wrap items-center gap-2 text-sm text-muted">
          A bearer token and custom headers, stored encrypted and never shown again.
          {current ? <CredentialsBadge server={server} /> : null}
        </p>
        {server.authMode === "oauth" ? (
          <p role="note" className="form-warning">
            Saving credentials ends the sign-in and forgets the OAuth client.
          </p>
        ) : null}
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
          {current && server.credentials !== "missing" ? (
            <Button variant="secondary" onClick={() => setRemoving(true)}>
              Remove credentials
            </Button>
          ) : null}
          <Button type="submit" busy={form.busy} disabled={form.wait > 0}>
            {current && server.credentials !== "missing"
              ? "Replace credentials"
              : "Save credentials"}
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
    </>
  );
}

/** Switches the server to `to`, dropping the old mode's credentials after a confirmation. */
function SwitchMode({ server, to }: { server: ToolServerResponse; to: AuthMode }) {
  const store = useStore();
  const [open, setOpen] = useState(false);
  const change = useMutation({
    mutationFn: () => updateToolServer(server.id, { authMode: to }),
    onSuccess: (saved) => {
      store(saved);
      setOpen(false);
    },
  });
  const label = to === "oauth" ? "Use sign-in" : "Use no credentials";
  const dropped =
    server.authMode === "headers"
      ? "The stored bearer token and headers are deleted."
      : "The sign-in ends and the OAuth client is forgotten.";

  return (
    <div className="space-y-3">
      <p className="text-sm text-muted">
        {to === "oauth"
          ? "Requests carry the access token of a sign-in at the server's sign-in page, renewed when it expires."
          : "Requests carry no credentials."}
      </p>
      <div className="flex justify-end">
        <Button onClick={() => setOpen(true)}>{label}</Button>
      </div>
      <ConfirmDialog
        open={open}
        title={`${label}?`}
        confirm={label}
        busy={change.isPending}
        error={change.error ? describeError(change.error).message : undefined}
        onConfirm={() => change.mutate()}
        onCancel={() => {
          setOpen(false);
          change.reset();
        }}
      >
        <p>{dropped}</p>
      </ConfirmDialog>
    </div>
  );
}

/** A sign-in started from this page: until when it may take, and how the server looked then. */
type Wait = { until: number; mark: string };

/**
 * An OAuth server's sign-in: its state, Sign in (the authorization page opens in the browser;
 * a native app then waits for the sign-in in the system browser and asks for the code that page
 * shows), Sign out, and an optional pre-registered client with the redirect URI to register for
 * it. Sign in stays available while a sign-in is pending, so a refused or abandoned one can be
 * started again at once.
 */
function SignIn({ server }: { server: ToolServerResponse }) {
  const store = useStore();
  const [params] = useSearchParams();
  const [notice, setNotice] = useState<string>();
  const [wait, setWait] = useState<Wait>();
  const state = server.oauth?.state ?? "needs_sign_in";
  const mark = signInMark(server);
  const waiting = wait !== undefined;
  useQuery({
    queryKey: keys.toolServer(server.id),
    queryFn: ({ signal }) => fetchToolServer(server.id, signal),
    enabled: waiting,
    refetchInterval: waiting ? SIGN_IN_POLL_MS : false,
  });
  useEffect(() => {
    if (wait === undefined) return;
    // Finished elsewhere (a browser signed in to Copilot sends the code itself).
    if (mark !== wait.mark) {
      setWait(undefined);
      if (state === "signed_in") setNotice("Signed in.");
      return;
    }
    const timer = window.setTimeout(() => setWait(undefined), wait.until - Date.now());
    return () => window.clearTimeout(timer);
  }, [wait, mark, state]);
  const start = useMutation({
    mutationFn: () => startSignIn(server.id),
    onMutate: () => setNotice(undefined),
    onSuccess: (started) => {
      if (isNative()) {
        setWait({ until: Date.now() + SIGN_IN_WAIT_MS, mark });
        setNotice(
          "Finish signing in in your browser, then enter the code it shows here. This page also updates by itself."
        );
      }
      openSignIn(started.authorizationUrl);
    },
  });
  const confirm = useSubmit(
    confirmSchema,
    ({ code }) => confirmSignIn(server.id, code),
    (saved) => {
      store(saved);
      setWait(undefined);
      setNotice("Signed in.");
    }
  );
  const out = useMutation({
    mutationFn: () => signOut(server.id),
    onMutate: () => setNotice(undefined),
    onSuccess: (saved) => {
      store(saved);
      setNotice("Signed out.");
    },
  });
  const client = useSubmit(
    clientSchema,
    (oauthClient) => updateToolServer(server.id, { oauthClient }),
    (saved) => {
      store(saved);
      setNotice(saved.oauth?.clientId ? "Client saved." : "Client removed.");
    }
  );
  const badge = signInBadge(state);
  const failure = start.error ?? out.error;
  const redirectUri = server.oauth?.redirectUri;

  return (
    <div className="space-y-4">
      <p className="flex flex-wrap items-center gap-2 text-sm text-muted">
        Requests carry the access token of your sign-in, renewed when it expires.
        <Badge tone={badge.tone}>{badge.label}</Badge>
      </p>
      <FormAlert>{failure ? describeError(failure).message : null}</FormAlert>
      <FormNotice>{notice}</FormNotice>
      <div className="flex flex-wrap justify-end gap-2">
        {state === "needs_sign_in" ? null : (
          <Button variant="secondary" busy={out.isPending} onClick={() => out.mutate()}>
            Sign out
          </Button>
        )}
        <Button busy={start.isPending} onClick={() => start.mutate()}>
          {start.isPending ? null : <LuLogIn aria-hidden />}
          {state === "signed_in" ? "Sign in again" : "Sign in"}
        </Button>
      </div>
      {waiting || params.has("code") ? (
        <form
          noValidate
          className="space-y-4"
          onSubmit={(event) => {
            event.preventDefault();
            setNotice(undefined);
            const data = new FormData(event.currentTarget);
            void confirm.submit({ code: text(data, "code") });
          }}
        >
          <FormAlert>{confirm.alert}</FormAlert>
          <TextField
            label="Code from the browser"
            name="code"
            autoComplete="one-time-code"
            autoCapitalize="characters"
            spellCheck={false}
            hint="The sign-in page in your browser shows it when you are done there, like ABCD-EFGH."
            error={confirm.errors.code}
          />
          <div className="flex justify-end">
            <Button type="submit" busy={confirm.busy} disabled={confirm.wait > 0}>
              Finish sign-in
            </Button>
          </div>
        </form>
      ) : null}
      <form
        key={server.updatedAt}
        noValidate
        className="space-y-4 border-t border-line pt-4"
        onSubmit={(event) => {
          event.preventDefault();
          setNotice(undefined);
          const data = new FormData(event.currentTarget);
          void client.submit({
            clientId: text(data, "clientId"),
            clientSecret: text(data, "clientSecret"),
          });
        }}
      >
        <FormAlert>{client.alert}</FormAlert>
        <ClientFields
          error={client.errors.clientId}
          defaultId={server.oauth?.clientId ?? undefined}
          secretStored={server.oauth?.clientSecret === "set"}
        />
        {redirectUri ? (
          <p className="text-sm text-muted">
            Redirect URI to register for this client:{" "}
            <code className="select-all break-all">{redirectUri}</code>
          </p>
        ) : null}
        <div className="flex justify-end">
          <Button variant="secondary" type="submit" busy={client.busy} disabled={client.wait > 0}>
            Save client
          </Button>
        </div>
      </form>
    </div>
  );
}

/** How the server's requests authenticate, with the chosen mode's controls. */
function Authentication({ server }: { server: ToolServerResponse }) {
  const [mode, setMode] = useState<AuthMode>(server.authMode);

  return (
    <Section
      title="Authentication"
      description="What Copilot sends to the server. Switching drops the other mode's credentials."
    >
      <div className="space-y-6">
        <AuthPicker value={mode} onChange={setMode} />
        {mode === "headers" ? <HeaderCredentials server={server} /> : null}
        {mode === "oauth" && server.authMode === "oauth" ? <SignIn server={server} /> : null}
        {mode === "oauth" && server.authMode !== "oauth" ? (
          <SwitchMode server={server} to="oauth" />
        ) : null}
        {mode === "none" && server.authMode !== "none" ? (
          <SwitchMode server={server} to="none" />
        ) : null}
        {mode === "none" && server.authMode === "none" ? (
          <p className="text-sm text-muted">Requests carry no credentials.</p>
        ) : null}
      </div>
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

/**
 * Settings, Tool servers, one server: its tools and their modes, settings, authentication
 * (credentials or sign-in) or a stdio server's program, delete.
 */
export function ToolServerDetails({ id }: { id: string }) {
  const state = useLocation().state as AddedState | null;
  const server = useQuery({
    queryKey: keys.toolServer(id),
    queryFn: ({ signal }) => fetchToolServer(id, signal),
  });
  const data = server.data;

  return (
    <SettingsPage
      title="Tool Server"
      heading={data?.name}
      description={data ? location(data) : undefined}
      back={BACK}
    >
      <QueryState query={server} back={BACK}>
        {data ? (
          <>
            <Tools server={data} state={state} />
            <Settings server={data} />
            {data.transport === "stdio" ? (
              <Program server={data} />
            ) : (
              <Authentication key={data.authMode} server={data} />
            )}
            <Remove server={data} />
          </>
        ) : null}
      </QueryState>
    </SettingsPage>
  );
}
