import type { Capability, ModelResponse, ReasoningLevel } from "@copilot/contracts";
import { describeError } from "@web/api/errors";
import Button from "@web/components/Button/Button";
import { Modal, useModalBusy } from "@web/components/Dialog/Modal";
import { FormAlert } from "@web/components/Form/FormAlert";
import {
  CAPABILITY_LABELS,
  capabilityNames,
  fitLevel,
  groupModels,
  levelLabel,
  modelSummary,
  reasoningLevels,
} from "@web/components/ModelPicker/models";
import path from "@web/routes/path";
import { type ReactNode, useId, useState } from "react";
import type { IconType } from "react-icons";
import {
  LuBrain,
  LuChevronDown,
  LuFileText,
  LuImage,
  LuMic,
  LuVolume2,
  LuWrench,
} from "react-icons/lu";
import { Link } from "react-router-dom";

const CAPABILITY_ICON_MAP: Record<Capability, IconType> = {
  vision: LuImage,
  pdf: LuFileText,
  tools: LuWrench,
  audioIn: LuMic,
  audioOut: LuVolume2,
  reasoning: LuBrain,
};

/** A model's capability icons, each named for screen readers and on hover. */
export function CapabilityIcons({ model }: { model: ModelResponse }) {
  const names = capabilityNames(model);
  if (names.length === 0) return null;
  return (
    <span className="inline-flex items-center gap-1 text-muted">
      {names.map((name) => {
        const Icon = CAPABILITY_ICON_MAP[name];
        const label = CAPABILITY_LABELS.find((item) => item.name === name)?.label ?? name;
        return (
          <span key={name} title={label} className="inline-flex">
            <Icon aria-hidden className="size-3.5" />
            <span className="sr-only">{label}</span>
          </span>
        );
      })}
    </span>
  );
}

type Choice = { modelId: string; reasoning: ReasoningLevel | null };

type ModelPickerProps = {
  /** `GET /models`: the enabled, available models. */
  models: ModelResponse[];
  /** The chosen model; `null` (or a model that is gone) asks for one. */
  modelId: string | null;
  reasoning: ReasoningLevel | null;
  /** Saves a choice; a rejected promise keeps the sheet open with the error. */
  onChange: (choice: Choice) => unknown;
  /** Tools are on for the chat, which turns reasoning off on some connections. */
  toolsOn?: boolean;
  disabled?: boolean;
};

/**
 * The model picker: a button with the chosen model and its capability icons, which opens a sheet
 * with the models grouped by connection and, for reasoning models, the reasoning level.
 */
export function ModelPicker({
  models,
  modelId,
  reasoning,
  onChange,
  toolsOn = false,
  disabled = false,
}: ModelPickerProps) {
  const [open, setOpen] = useState(false);
  const current = models.find((model) => model.id === modelId);
  const label = current
    ? current.displayName
    : models.length === 0
      ? "No models"
      : "Choose a model";

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        disabled={disabled}
        aria-haspopup="dialog"
        className="inline-flex min-h-10 max-w-full items-center gap-2 rounded-lg px-3 text-sm font-medium hover:bg-raised disabled:opacity-60"
      >
        <span className="sr-only">Model: </span>
        <span className="truncate">{label}</span>
        {current?.capabilities.reasoning ? (
          <span className="shrink-0 text-xs text-muted">
            {levelLabel(fitLevel(current, reasoning), current)}
          </span>
        ) : null}
        {current ? <CapabilityIcons model={current} /> : null}
        <LuChevronDown aria-hidden className="shrink-0 text-muted" />
      </button>
      <Modal open={open} title="Choose a Model" onClose={() => setOpen(false)}>
        <Sheet
          models={models}
          initial={{ modelId: current?.id ?? "", reasoning: fitLevel(current, reasoning) }}
          toolsOn={toolsOn}
          onCancel={() => setOpen(false)}
          onPick={async (choice) => {
            await onChange(choice);
            setOpen(false);
          }}
        />
      </Modal>
    </>
  );
}

function Option({ name, value, checked, onChange, children }: OptionProps) {
  return (
    <label className="choice-card items-start gap-3">
      <input
        type="radio"
        name={name}
        value={value}
        checked={checked}
        onChange={onChange}
        className="mt-1 shrink-0"
      />
      <span className="min-w-0 flex-1">{children}</span>
    </label>
  );
}

type OptionProps = {
  name: string;
  value: string;
  checked: boolean;
  onChange: () => void;
  children: ReactNode;
};

type SheetProps = {
  models: ModelResponse[];
  initial: Choice;
  toolsOn: boolean;
  onCancel: () => void;
  onPick: (choice: Choice) => Promise<void>;
};

/** The sheet's form: one radio group per connection, then the reasoning levels. */
export function Sheet({ models, initial, toolsOn, onCancel, onPick }: SheetProps) {
  const [choice, setChoice] = useState<Choice>(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const id = useId();
  const groups = groupModels(models);
  useModalBusy(busy);
  const model = models.find((item) => item.id === choice.modelId);
  const levels = model ? reasoningLevels(model) : [];

  if (groups.length === 0) {
    return (
      <p className="text-sm text-muted">
        No models yet.{" "}
        <Link to={path.welcome} className="text-link">
          Add a provider
        </Link>{" "}
        or turn on models in{" "}
        <Link to={path.providers} className="text-link">
          Settings
        </Link>
        .
      </p>
    );
  }

  return (
    <form
      className="space-y-5"
      onSubmit={async (event) => {
        event.preventDefault();
        if (!model) return;
        setBusy(true);
        setError(undefined);
        try {
          await onPick({ modelId: model.id, reasoning: fitLevel(model, choice.reasoning) });
        } catch (failure) {
          setError(describeError(failure).message);
        } finally {
          setBusy(false);
        }
      }}
    >
      {groups.map((group) => (
        <fieldset key={group.providerId} className="space-y-2">
          <legend className="mb-2 text-xs font-medium uppercase tracking-wider text-muted">
            {group.provider}
          </legend>
          {group.models.map((item) => (
            <Option
              key={item.id}
              name={`${id}-model`}
              value={item.id}
              checked={item.id === choice.modelId}
              onChange={() => setChoice({ modelId: item.id, reasoning: choice.reasoning })}
            >
              <span className="flex flex-wrap items-center gap-2">
                <span className="font-medium">{item.displayName}</span>
                <CapabilityIcons model={item} />
              </span>
              <span className="block text-xs text-muted">{modelSummary(item)}</span>
            </Option>
          ))}
        </fieldset>
      ))}

      {model && levels.length > 0 ? (
        <fieldset className="space-y-2">
          <legend className="mb-2 text-xs font-medium uppercase tracking-wider text-muted">
            Reasoning
          </legend>
          {toolsOn && model.reasoningOffWithTools ? (
            <p className="form-warning">
              This connection turns reasoning off while tools are on in the chat.
            </p>
          ) : null}
          <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
            {[null, ...levels].map((level) => (
              <Option
                key={level ?? "default"}
                name={`${id}-reasoning`}
                value={level ?? ""}
                checked={fitLevel(model, choice.reasoning) === level}
                onChange={() => setChoice({ modelId: choice.modelId, reasoning: level })}
              >
                {levelLabel(level, model)}
              </Option>
            ))}
          </div>
        </fieldset>
      ) : null}

      <FormAlert>{error}</FormAlert>
      <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
        <Button variant="secondary" onClick={onCancel} disabled={busy}>
          Cancel
        </Button>
        <Button type="submit" busy={busy} disabled={!model}>
          Use model
        </Button>
      </div>
    </form>
  );
}
