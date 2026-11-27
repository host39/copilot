import type { ProviderApi } from "@copilot/contracts";

type ApiPickerProps = {
  /** The API checked when the form opens. */
  defaultValue: ProviderApi;
};

const OPTIONS: { value: ProviderApi; label: string; hint: string }[] = [
  {
    value: "responses",
    label: "Responses API",
    hint: "Tool calls and reasoning summaries on OpenAI's newest models. No audio.",
  },
  {
    value: "chat_completions",
    label: "Chat Completions",
    hint: "Every compatible server has it. Audio models need it.",
  },
];

/** The label of an OpenAI API, as the connection's page shows it. */
export function apiLabel(api: ProviderApi): string {
  return OPTIONS.find((option) => option.value === api)?.label ?? api;
}

/**
 * Which OpenAI API a connection's answers use (`api` in the form's data). Only the OpenAI and
 * OpenAI-compatible presets show it; the server refuses Responses for the others.
 */
export function ApiPicker({ defaultValue }: ApiPickerProps) {
  return (
    <fieldset>
      <legend className="field-label">API</legend>
      <div className="mt-2 grid gap-2 sm:grid-cols-2">
        {OPTIONS.map((option) => (
          <label key={option.value} className="choice-card flex-col gap-0.5">
            <input
              type="radio"
              name="api"
              value={option.value}
              defaultChecked={defaultValue === option.value}
              className="sr-only"
            />
            <span className="font-medium">{option.label}</span>
            <span className="text-muted">{option.hint}</span>
          </label>
        ))}
      </div>
    </fieldset>
  );
}
