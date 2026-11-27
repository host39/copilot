import type { ModelResponse } from "@copilot/contracts";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { keys } from "@web/api/routes/keys";
import { updateModel } from "@web/api/routes/models";
import { Badge } from "@web/components/Badge/Badge";
import Button from "@web/components/Button/Button";
import { FormAlert } from "@web/components/Form/FormAlert";
import { Switch } from "@web/components/Form/Switch";
import { TextField } from "@web/components/Form/TextField";
import { useSubmit } from "@web/hooks/useSubmit";
import {
  CAPABILITIES,
  modelSchema,
  overrideChoice,
  shortTokens,
} from "@web/routes/settings/provider/models";
import { useId, useState } from "react";
import { LuChevronDown, LuChevronUp } from "react-icons/lu";

/** Stores a saved model in its connection's list and refreshes the model picker. */
function useSaved(model: ModelResponse) {
  const client = useQueryClient();
  return (saved: ModelResponse) => {
    client.setQueryData<ModelResponse[]>(keys.providerModels(model.providerId), (list) =>
      list?.map((item) => (item.id === saved.id ? saved : item))
    );
    void client.invalidateQueries({ queryKey: keys.models });
  };
}

function field(data: FormData, name: string): string {
  const value = data.get(name);
  return typeof value === "string" ? value : "";
}

function Editor({ model, onDone }: { model: ModelResponse; onDone: () => void }) {
  const saved = useSaved(model);
  const locks = new Map(model.locks.map((lock) => [lock.capability, lock.reason]));
  const form = useSubmit(
    modelSchema(model),
    async (body) => (Object.keys(body).length === 0 ? model : updateModel(model.id, body)),
    (result) => {
      saved(result);
      onDone();
    }
  );

  return (
    <form
      noValidate
      className="mt-3 space-y-4 rounded-xl border border-line bg-surface p-4"
      onSubmit={(event) => {
        event.preventDefault();
        const data = new FormData(event.currentTarget);
        const input: Record<string, string> = {
          displayName: field(data, "displayName"),
          contextWindow: field(data, "contextWindow"),
          maxOutputTokens: field(data, "maxOutputTokens"),
        };
        for (const { name } of CAPABILITIES) {
          const value = data.get(name);
          if (typeof value === "string") input[name] = value;
        }
        void form.submit(input);
      }}
    >
      <FormAlert>{form.alert ?? form.errors.capabilityOverrides}</FormAlert>
      <TextField
        label="Display name"
        name="displayName"
        defaultValue={model.displayName}
        autoComplete="off"
        error={form.errors.displayName}
      />
      <div className="grid gap-4 sm:grid-cols-2">
        <TextField
          label="Context window (tokens)"
          name="contextWindow"
          inputMode="numeric"
          autoComplete="off"
          defaultValue={model.contextWindow ?? ""}
          placeholder="Not reported"
          hint="Sets the context meter and the project capacity bar."
          error={form.errors.contextWindow}
        />
        <TextField
          label="Max output tokens"
          name="maxOutputTokens"
          inputMode="numeric"
          autoComplete="off"
          defaultValue={model.maxOutputTokens ?? ""}
          placeholder="8192"
          hint="Leave it empty for 8,192 unless the provider reports a limit."
          error={form.errors.maxOutputTokens}
        />
      </div>
      <fieldset>
        <legend className="field-label">Capabilities</legend>
        <p className="field-hint">
          Detected from what the provider reports. Override a flag when it is wrong.
        </p>
        <div className="mt-3 grid gap-3 sm:grid-cols-2">
          {CAPABILITIES.map(({ name, label }) => {
            const reason = locks.get(name);
            if (reason !== undefined) {
              return (
                <div key={name}>
                  <p className="text-sm font-medium">{label}</p>
                  <p className="text-sm text-muted">
                    {model.capabilities[name] ? "On" : "Off"}. {reason}
                  </p>
                </div>
              );
            }
            return (
              <label key={name} className="block">
                <span className="field-label">{label}</span>
                <select
                  name={name}
                  defaultValue={overrideChoice(model, name)}
                  className="field-input"
                >
                  <option value="default">Detected ({model.detected[name] ? "on" : "off"})</option>
                  <option value="on">On</option>
                  <option value="off">Off</option>
                </select>
              </label>
            );
          })}
        </div>
      </fieldset>
      <div className="flex flex-wrap justify-end gap-2">
        <Button variant="secondary" onClick={onDone}>
          Cancel
        </Button>
        <Button type="submit" busy={form.busy}>
          Save model
        </Button>
      </div>
    </form>
  );
}

/** What the model can do, as short labels. */
function Traits({ model }: { model: ModelResponse }) {
  const traits = CAPABILITIES.filter(({ name }) => model.capabilities[name]).map(
    ({ label }) => label
  );
  if (model.contextWindow !== null) traits.push(`${shortTokens(model.contextWindow)} context`);
  if (traits.length === 0) return null;
  return <p className="mt-1 text-xs text-muted">{traits.join(" · ")}</p>;
}

/**
 * One model of a connection: the enable switch, its name, id, and capabilities, and an editor for
 * its name, token limits, and capability overrides.
 */
export function ModelRow({
  model,
  editable = true,
}: {
  model: ModelResponse;
  /** The first-run setup only turns models on and off. */
  editable?: boolean;
}) {
  const saved = useSaved(model);
  const [open, setOpen] = useState(false);
  const editor = useId();
  const toggle = useMutation({
    mutationFn: (enabled: boolean) => updateModel(model.id, { enabled }),
    onSuccess: saved,
  });
  const enabled = toggle.isPending ? (toggle.variables ?? model.enabled) : model.enabled;

  return (
    <li className="py-3">
      <div className="flex items-start gap-3">
        <Switch
          label={`Use ${model.displayName}`}
          hideLabel
          checked={enabled}
          disabled={toggle.isPending}
          onChange={(event) => toggle.mutate(event.target.checked)}
          className="pt-0.5"
        />
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-center gap-2 text-sm font-medium">
            <span className="break-all">{model.displayName}</span>
            {model.available ? null : <Badge tone="warn">No longer listed</Badge>}
          </p>
          {model.remoteId === model.displayName ? null : (
            <p className="break-all text-xs text-muted">{model.remoteId}</p>
          )}
          <Traits model={model} />
        </div>
        {editable ? (
          <Button
            variant="ghost"
            aria-expanded={open}
            aria-controls={open ? editor : undefined}
            aria-label={`Edit ${model.displayName}`}
            onClick={() => setOpen((value) => !value)}
            className="px-3"
          >
            <span className="hidden sm:inline">Edit</span>
            {open ? <LuChevronUp aria-hidden /> : <LuChevronDown aria-hidden />}
          </Button>
        ) : null}
      </div>
      <FormAlert>{toggle.error ? describeError(toggle.error).message : null}</FormAlert>
      {open ? (
        <div id={editor}>
          <Editor model={model} onDone={() => setOpen(false)} />
        </div>
      ) : null}
    </li>
  );
}
