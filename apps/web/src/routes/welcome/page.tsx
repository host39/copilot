import { useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { keys } from "@web/api/routes/keys";
import { fetchProviderModels, refreshProvider } from "@web/api/routes/providers";
import { useUser } from "@web/auth/session";
import Button from "@web/components/Button/Button";
import { FormAlert } from "@web/components/Form/FormAlert";
import { PageTitle } from "@web/components/Layout/PageTitle";
import { ModelRow } from "@web/components/Settings/ModelRow";
import { ProviderFields, providerValues } from "@web/components/Settings/ProviderFields";
import { QueryState } from "@web/components/Settings/QueryState";
import { useSubmit } from "@web/hooks/useSubmit";
import path from "@web/routes/path";
import { filterModels } from "@web/routes/settings/provider/models";
import { providerSchema, refreshSummary } from "@web/routes/settings/providers/presets";
import {
  AUTO_ENABLE_MAX,
  firstModel,
  initialSetup,
  keepsKey,
  modelsNote,
  type SetupEvent,
  type SetupState,
  type StartState,
  saveConnection,
  setupReducer,
} from "@web/routes/welcome/setup";
import { type Dispatch, useEffect, useReducer, useRef, useState } from "react";
import { LuCheck, LuLoaderCircle, LuSearch } from "react-icons/lu";
import { Link, useNavigate } from "react-router-dom";

const STEPS = ["Connect a provider", "Test", "Choose models"] as const;

/** Models shown at a time in step 3. */
const MODELS_PAGE = 50;

function Steps({ current }: { current: number }) {
  return (
    <ol className="flex gap-2 text-sm">
      {STEPS.map((label, index) => {
        const done = index < current;
        const active = index === current;
        return (
          <li
            key={label}
            aria-current={active ? "step" : undefined}
            className={`flex flex-1 flex-col gap-2 ${active ? "text-fg" : "text-muted"}`}
          >
            <span
              className={`h-1 rounded-full ${done || active ? "bg-accent" : "bg-line"}`}
              aria-hidden
            />
            <span className="flex items-center gap-1">
              {done ? <LuCheck aria-hidden className="text-accent" /> : null}
              <span className={active ? "font-medium" : ""}>
                {label}
                {done ? <span className="sr-only"> (done)</span> : null}
              </span>
            </span>
          </li>
        );
      })}
    </ol>
  );
}

type StepProps = { state: SetupState; dispatch: Dispatch<SetupEvent> };

/** Tests the saved connection by listing its models. */
function useTest(dispatch: StepProps["dispatch"]) {
  const client = useQueryClient();
  return async (providerId: string) => {
    try {
      const report = await refreshProvider(providerId);
      void client.invalidateQueries({ queryKey: keys.providers });
      void client.invalidateQueries({ queryKey: keys.models });
      dispatch({ type: "tested", report });
    } catch (error) {
      dispatch({ type: "failed", problem: describeError(error).message });
    }
  };
}

function Connect({ state, dispatch }: StepProps) {
  const client = useQueryClient();
  const test = useTest(dispatch);
  const keep = keepsKey(state);
  const form = useSubmit(
    providerSchema(state.pick, keep),
    (body) => saveConnection(state.provider, body),
    (provider) => {
      void client.invalidateQueries({ queryKey: keys.providers });
      dispatch({ type: "saved", provider });
      void test(provider.id);
    }
  );

  return (
    <form
      noValidate
      className="space-y-6"
      onSubmit={(event) => {
        event.preventDefault();
        void form.submit(providerValues(new FormData(event.currentTarget)));
      }}
    >
      <FormAlert>{form.alert}</FormAlert>
      {keep ? (
        <p className="text-sm text-muted">Leave the key empty to keep the one you entered.</p>
      ) : null}
      <ProviderFields
        pick={state.pick}
        onPick={(pick) => dispatch({ type: "pick", pick })}
        errors={form.errors}
        askName={false}
        baseUrl={
          state.provider?.kind === state.pick.kind && state.provider.preset === state.pick.preset
            ? state.provider.baseUrl
            : undefined
        }
      />
      <div className="flex justify-end">
        <Button type="submit" busy={form.busy} disabled={form.wait > 0}>
          Continue
        </Button>
      </div>
    </form>
  );
}

function Test({ state, dispatch }: StepProps) {
  const test = useTest(dispatch);
  const heading = useRef<HTMLHeadingElement>(null);
  const status = state.test.status;

  // Move focus to the outcome, so screen reader and keyboard users land on it.
  useEffect(() => {
    if (status === "passed" || status === "failed") heading.current?.focus();
  }, [status]);

  return (
    <div className="space-y-4">
      <h2 ref={heading} tabIndex={-1} className="text-base font-semibold outline-none">
        {status === "passed"
          ? "Connected"
          : status === "failed"
            ? "The test failed"
            : `Testing ${state.pick.label}`}
      </h2>
      {status === "running" ? (
        <p className="flex items-center gap-2 text-sm text-muted" role="status">
          <LuLoaderCircle aria-hidden className="size-4 animate-spin" />
          Asking the provider for its models.
        </p>
      ) : null}
      {state.test.status === "passed" ? (
        <p role="status" className="text-sm">
          {refreshSummary(state.test.report)}
        </p>
      ) : null}
      {state.test.status === "failed" ? (
        <FormAlert>{`${state.test.problem} Go back to check the key or address, then test again.`}</FormAlert>
      ) : null}
      <div className="flex flex-wrap justify-between gap-2">
        <Button
          variant="secondary"
          disabled={status === "running"}
          onClick={() => dispatch({ type: "back" })}
        >
          Back
        </Button>
        {status === "failed" || status === "idle" ? (
          <Button
            onClick={() => {
              if (!state.provider) return;
              dispatch({ type: "retest" });
              void test(state.provider.id);
            }}
          >
            {status === "idle" ? "Test connection" : "Test again"}
          </Button>
        ) : (
          <Button disabled={status !== "passed"} onClick={() => dispatch({ type: "next" })}>
            Continue
          </Button>
        )}
      </div>
    </div>
  );
}

function Models({ state, dispatch }: StepProps) {
  const providerId = state.provider?.id ?? "";
  const navigate = useNavigate();
  const models = useQuery({
    queryKey: keys.providerModels(providerId),
    queryFn: ({ signal }) => fetchProviderModels(providerId, signal),
    enabled: providerId !== "",
  });
  const [query, setQuery] = useState("");
  const [limit, setLimit] = useState(MODELS_PAGE);
  const all = models.data ?? [];
  const shown = filterModels(all, query, "all");
  const first = firstModel(all);

  return (
    <div className="space-y-4">
      <h2 className="text-base font-semibold">Choose models</h2>
      {models.isSuccess && all.length > 0 ? (
        <p className="text-sm text-muted">{modelsNote(all.length)}</p>
      ) : null}
      <QueryState query={models}>
        {all.length > AUTO_ENABLE_MAX ? (
          <div className="relative">
            <label htmlFor="setup-search" className="sr-only">
              Search models
            </label>
            <LuSearch
              aria-hidden
              className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-muted"
            />
            <input
              id="setup-search"
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
        ) : null}
        {all.length === 0 ? (
          <p className="text-sm text-muted">The provider listed no models.</p>
        ) : (
          <ul className="max-h-[50dvh] divide-y divide-line overflow-y-auto">
            {shown.slice(0, limit).map((model) => (
              <ModelRow key={model.id} model={model} editable={false} />
            ))}
          </ul>
        )}
        {shown.length > limit ? (
          <Button variant="secondary" onClick={() => setLimit(limit + MODELS_PAGE)}>
            Show more ({shown.length - limit} left)
          </Button>
        ) : null}
      </QueryState>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <Button variant="secondary" onClick={() => dispatch({ type: "back" })}>
          Back
        </Button>
        {models.isSuccess && all.length > 0 && first === undefined ? (
          <p id="setup-start-hint" className="ml-auto text-sm text-muted">
            Turn on a model to start.
          </p>
        ) : null}
        <Button
          disabled={first === undefined}
          aria-describedby={first === undefined ? "setup-start-hint" : undefined}
          onClick={() => {
            const start: StartState = { modelId: first?.id };
            navigate(path.home, { replace: true, state: start });
          }}
        >
          Start chatting
        </Button>
      </div>
    </div>
  );
}

/**
 * First-run setup: (1) pick a preset and paste a key, or enter an Ollama address; (2) test the
 * connection by listing its models; (3) choose models (all turned on when the provider lists 20 or
 * fewer). It ends in a new chat with the first model turned on.
 */
export default function WelcomePage() {
  const user = useUser();
  const [state, dispatch] = useReducer(setupReducer, initialSetup);

  return (
    <main className="min-h-dvh bg-surface px-4 py-8 sm:py-12">
      <PageTitle page="Welcome" />
      <div className="mx-auto w-full max-w-xl">
        <div className="flex items-center justify-between gap-4">
          <div className="flex items-center gap-2">
            <img src="/icons/icon.svg" alt="" className="size-8" />
            <span className="font-semibold tracking-tight">Copilot</span>
          </div>
          <Link to={path.home} className="text-link inline-flex min-h-10 items-center text-sm">
            Skip for now
          </Link>
        </div>

        <h1 className="mt-8 text-2xl font-semibold tracking-tight">{`Welcome, ${user.displayName}`}</h1>
        <p className="mt-2 text-sm text-muted">
          Copilot forwards your chats to a model provider. Connect one to start.
        </p>

        <div className="mt-6">
          <Steps current={state.step} />
        </div>

        <div className="card-panel mt-6">
          {state.step === 0 ? <Connect state={state} dispatch={dispatch} /> : null}
          {state.step === 1 ? <Test state={state} dispatch={dispatch} /> : null}
          {state.step === 2 ? <Models state={state} dispatch={dispatch} /> : null}
        </div>
      </div>
    </main>
  );
}
