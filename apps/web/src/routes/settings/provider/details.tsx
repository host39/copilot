import type { ProviderDetailsResponse, ProviderResponse } from "@copilot/contracts";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { keys } from "@web/api/routes/keys";
import {
  deleteProvider,
  fetchProvider,
  fetchProviderModels,
  refreshProvider,
  updateProvider,
} from "@web/api/routes/providers";
import { Badge } from "@web/components/Badge/Badge";
import Button from "@web/components/Button/Button";
import { ConfirmDialog } from "@web/components/Dialog/ConfirmDialog";
import { FormAlert } from "@web/components/Form/FormAlert";
import { FormNotice } from "@web/components/Form/FormNotice";
import { Switch } from "@web/components/Form/Switch";
import { TextField } from "@web/components/Form/TextField";
import { ModelRow } from "@web/components/Settings/ModelRow";
import { QueryState } from "@web/components/Settings/QueryState";
import { Section } from "@web/components/Settings/Section";
import { SettingsPage } from "@web/components/Settings/SettingsPage";
import { text } from "@web/forms";
import { useSubmit } from "@web/hooks/useSubmit";
import path from "@web/routes/path";
import { connectionSchema, dropsKey } from "@web/routes/settings/provider/connection";
import { filterModels, type Show, usageText } from "@web/routes/settings/provider/models";
import { type ConnectState, choice, refreshSummary } from "@web/routes/settings/providers/presets";
import { useState } from "react";
import { LuRefreshCw, LuSearch } from "react-icons/lu";
import { useLocation, useNavigate } from "react-router-dom";

/** Models shown at a time; "Show more" adds this many. */
const MODELS_PAGE = 50;

const BACK = { to: path.providers, label: "Providers" };

/** Marks the connection's details, the provider list, and the model picker as stale. */
function useStale() {
  const client = useQueryClient();
  return () => {
    void client.invalidateQueries({ queryKey: keys.providers });
    void client.invalidateQueries({ queryKey: keys.models });
  };
}

/** Shows a saved connection at once (its details keep their usage counts), then refetches. */
function useStore(id: string) {
  const client = useQueryClient();
  const stale = useStale();
  return (saved: ProviderResponse) => {
    client.setQueryData<ProviderDetailsResponse>(
      keys.provider(id),
      (old) => old && { ...old, ...saved }
    );
    stale();
  };
}

function KeyBadge({ provider }: { provider: ProviderResponse }) {
  switch (provider.key) {
    case "set":
      return <Badge tone="ok">Key stored</Badge>;
    case "unreadable":
      return <Badge tone="danger">Key unreadable: enter it again</Badge>;
    default:
      return <Badge>No key</Badge>;
  }
}

function Connection({ provider }: { provider: ProviderResponse }) {
  const store = useStore(provider.id);
  const [notice, setNotice] = useState<string>();
  const [removing, setRemoving] = useState(false);
  const schema = connectionSchema(provider);
  const form = useSubmit(
    schema,
    async (body) => {
      if (Object.keys(body).length === 0) return { saved: provider, dropped: false };
      const saved = await updateProvider(provider.id, body);
      return { saved, dropped: dropsKey(provider, body) };
    },
    (result) => {
      store(result.saved);
      setNotice(
        result.dropped
          ? "Saved. The key was removed because the address moved to another server: enter it again."
          : "Saved."
      );
    }
  );
  const enable = useMutation({
    mutationFn: (enabled: boolean) => updateProvider(provider.id, { enabled }),
    onSuccess: store,
  });
  const removeKey = useMutation({
    mutationFn: () => updateProvider(provider.id, { apiKey: "" }),
    onSuccess: (result) => {
      store(result);
      setRemoving(false);
      setNotice("The key was removed.");
    },
  });
  const enabled = enable.isPending ? (enable.variables ?? provider.enabled) : provider.enabled;

  return (
    <Section
      title="Connection"
      description={
        <span className="flex flex-wrap items-center gap-2">
          {choice(provider.kind, provider.preset).label}
          <KeyBadge provider={provider} />
        </span>
      }
    >
      <div className="space-y-6">
        <Switch
          label="Use this provider"
          hint="When it is off, its models leave the model picker."
          checked={enabled}
          disabled={enable.isPending}
          onChange={(event) => enable.mutate(event.target.checked)}
        />
        <FormAlert>{enable.error ? describeError(enable.error).message : null}</FormAlert>
        <form
          key={provider.updatedAt}
          noValidate
          className="space-y-4"
          onSubmit={(event) => {
            event.preventDefault();
            setNotice(undefined);
            const data = new FormData(event.currentTarget);
            void form.submit({
              name: text(data, "name"),
              baseUrl: text(data, "baseUrl"),
              apiKey: text(data, "apiKey"),
            });
          }}
        >
          <FormAlert>{form.alert}</FormAlert>
          <FormNotice>{notice}</FormNotice>
          <TextField
            label="Name"
            name="name"
            autoComplete="off"
            defaultValue={provider.name}
            error={form.errors.name}
          />
          <TextField
            label="Server address"
            name="baseUrl"
            type="url"
            inputMode="url"
            autoComplete="off"
            autoCapitalize="none"
            spellCheck={false}
            defaultValue={provider.baseUrl}
            hint="Moving to another host removes the stored key unless you enter a new one."
            error={form.errors.baseUrl}
          />
          <TextField
            label={provider.key === "missing" ? "API key" : "New API key"}
            name="apiKey"
            type="password"
            autoComplete="off"
            spellCheck={false}
            placeholder={provider.key === "set" ? "Leave empty to keep the stored key" : undefined}
            hint="Stored encrypted on your server. It is never shown again."
            error={form.errors.apiKey}
          />
          <div className="flex flex-wrap justify-end gap-2">
            {provider.key === "missing" ? null : (
              <Button variant="secondary" onClick={() => setRemoving(true)}>
                Remove key
              </Button>
            )}
            <Button type="submit" busy={form.busy} disabled={form.wait > 0}>
              Save connection
            </Button>
          </div>
        </form>
      </div>
      <ConfirmDialog
        open={removing}
        title="Remove the API key?"
        confirm="Remove key"
        busy={removeKey.isPending}
        error={removeKey.error ? describeError(removeKey.error).message : undefined}
        onConfirm={() => removeKey.mutate()}
        onCancel={() => {
          setRemoving(false);
          removeKey.reset();
        }}
      >
        <p>Chats with this provider's models fail until you enter a key again.</p>
      </ConfirmDialog>
    </Section>
  );
}

function Models({ provider, state }: { provider: ProviderResponse; state: ConnectState | null }) {
  const stale = useStale();
  const models = useQuery({
    queryKey: keys.providerModels(provider.id),
    queryFn: ({ signal }) => fetchProviderModels(provider.id, signal),
  });
  const [notice, setNotice] = useState(state?.refresh ? refreshSummary(state.refresh) : undefined);
  const [query, setQuery] = useState("");
  const [show, setShow] = useState<Show>("all");
  const [limit, setLimit] = useState(MODELS_PAGE);
  const refresh = useMutation({
    mutationFn: () => refreshProvider(provider.id),
    onMutate: () => setNotice(undefined),
    onSuccess: (report) => {
      setNotice(refreshSummary(report));
      stale();
    },
  });
  const all = models.data ?? [];
  const shown = filterModels(all, query, show);
  const enabled = all.filter((model) => model.enabled).length;
  const problem = refresh.error
    ? describeError(refresh.error).message
    : refresh.isIdle
      ? state?.problem
      : undefined;

  return (
    <Section
      title="Models"
      description="Turn on the models you want in the model picker."
      actions={
        <Button variant="secondary" busy={refresh.isPending} onClick={() => refresh.mutate()}>
          {refresh.isPending ? null : <LuRefreshCw aria-hidden />}
          Test and refresh
        </Button>
      }
    >
      <div className="space-y-4">
        <FormAlert>
          {problem
            ? `The connection test failed: ${problem} Fix the connection, then test again.`
            : null}
        </FormAlert>
        <FormNotice>{notice}</FormNotice>
        <QueryState query={models}>
          {all.length === 0 ? (
            <p className="text-sm text-muted">No models yet. Test the connection to list them.</p>
          ) : (
            <>
              <div className="flex flex-col gap-2 sm:flex-row">
                <div className="relative flex-1">
                  <label htmlFor="model-search" className="sr-only">
                    Search models
                  </label>
                  <LuSearch
                    aria-hidden
                    className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-muted"
                  />
                  <input
                    id="model-search"
                    type="search"
                    value={query}
                    placeholder="Search models"
                    autoComplete="off"
                    onChange={(event) => {
                      setQuery(event.target.value);
                      setLimit(MODELS_PAGE);
                    }}
                    className="field-input mt-0 pl-9"
                  />
                </div>
                <div className="sm:w-44">
                  <label htmlFor="model-show" className="sr-only">
                    Show
                  </label>
                  <select
                    id="model-show"
                    value={show}
                    onChange={(event) => {
                      setShow(event.target.value as Show);
                      setLimit(MODELS_PAGE);
                    }}
                    className="field-input mt-0"
                  >
                    <option value="all">All models</option>
                    <option value="enabled">Turned on</option>
                    <option value="disabled">Turned off</option>
                  </select>
                </div>
              </div>
              <p className="text-sm text-muted" aria-live="polite">
                {`${enabled} of ${all.length} turned on${shown.length === all.length ? "" : `, ${shown.length} shown`}`}
              </p>
              {shown.length === 0 ? (
                <p className="text-sm text-muted">No model matches.</p>
              ) : (
                <ul className="divide-y divide-line">
                  {shown.slice(0, limit).map((model) => (
                    <ModelRow key={model.id} model={model} />
                  ))}
                </ul>
              )}
              {shown.length > limit ? (
                <div className="flex justify-center">
                  <Button variant="secondary" onClick={() => setLimit(limit + MODELS_PAGE)}>
                    Show more ({shown.length - limit} left)
                  </Button>
                </div>
              ) : null}
            </>
          )}
        </QueryState>
      </div>
    </Section>
  );
}

function Remove({ provider, reload }: { provider: ProviderDetailsResponse; reload: () => void }) {
  const client = useQueryClient();
  const navigate = useNavigate();
  const [open, setOpen] = useState(false);
  const remove = useMutation({
    mutationFn: () => deleteProvider(provider.id),
    onSuccess: () => {
      client.removeQueries({ queryKey: keys.provider(provider.id) });
      void client.invalidateQueries({ queryKey: keys.providers });
      void client.invalidateQueries({ queryKey: keys.models });
      navigate(path.providers, { replace: true });
    },
  });

  return (
    <Section title="Delete Provider" description="Removes the connection and all of its models.">
      <Button
        variant="danger"
        onClick={() => {
          reload();
          setOpen(true);
        }}
      >
        Delete provider
      </Button>
      <ConfirmDialog
        open={open}
        title={`Delete ${provider.name}?`}
        confirm="Delete provider"
        busy={remove.isPending}
        error={remove.error ? describeError(remove.error).message : undefined}
        onConfirm={() => remove.mutate()}
        onCancel={() => {
          setOpen(false);
          remove.reset();
        }}
      >
        <p>{usageText(provider.usage)}</p>
        <p>This cannot be undone.</p>
      </ConfirmDialog>
    </Section>
  );
}

/** Settings, Providers, one connection: its settings, its models, and delete. */
export function ProviderDetails({ id }: { id: string }) {
  const state = useLocation().state as ConnectState | null;
  const details = useQuery({
    queryKey: keys.provider(id),
    queryFn: ({ signal }) => fetchProvider(id, signal),
  });
  const provider = details.data;

  return (
    <SettingsPage title="Provider" heading={provider?.name} back={BACK}>
      <QueryState query={details} back={BACK}>
        {provider ? (
          <>
            <Connection provider={provider} />
            <Models provider={provider} state={state} />
            <Remove provider={provider} reload={() => void details.refetch()} />
          </>
        ) : null}
      </QueryState>
    </SettingsPage>
  );
}
