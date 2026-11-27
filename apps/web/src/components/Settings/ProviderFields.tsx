import { TextField } from "@web/components/Form/TextField";
import { ApiPicker } from "@web/components/Settings/ApiPicker";
import { text } from "@web/forms";
import { CHOICES, type Choice } from "@web/routes/settings/providers/presets";

type ProviderFieldsProps = {
  pick: Choice;
  onPick: (pick: Choice) => void;
  errors: Record<string, string>;
  /** The first-run setup names the connection after its preset without asking. */
  askName?: boolean;
  /** The address of a connection being fixed (the first-run setup's Back). */
  baseUrl?: string;
};

function addressHint(pick: Choice): string {
  if (pick.asks === "url") {
    return `Leave it empty to use ${pick.urlHint} on the Copilot server.`;
  }
  return pick.kind === "anthropic"
    ? "The address in front of /v1/messages."
    : "The address in front of /chat/completions, usually ending in /v1.";
}

/**
 * The add-provider fields of Settings and the first-run setup: the preset as a radio group, then
 * what that preset needs (a key, an address, or both, and the OpenAI API where the preset lets the
 * user choose). Each field is keyed by the preset, so switching presets never carries a key over
 * to another provider.
 */
export function ProviderFields({
  pick,
  onPick,
  errors,
  askName = true,
  baseUrl,
}: ProviderFieldsProps) {
  return (
    <>
      <fieldset>
        <legend className="field-label">Provider</legend>
        <div className="mt-2 grid grid-cols-2 gap-2 sm:grid-cols-3">
          {CHOICES.map((option) => (
            <label key={option.id} className="choice-card min-h-12 items-center">
              <input
                type="radio"
                name="preset"
                value={option.id}
                checked={pick.id === option.id}
                onChange={() => onPick(option)}
                className="sr-only"
              />
              {option.label}
            </label>
          ))}
        </div>
      </fieldset>

      {askName ? (
        <TextField
          key={`${pick.id}-name`}
          label="Name (optional)"
          name="name"
          autoComplete="off"
          placeholder={pick.label}
          hint="Shown in the model picker. Leave it empty to use the provider's name."
          error={errors.name}
        />
      ) : null}
      {pick.asks !== "key" ? (
        <TextField
          key={`${pick.id}-url`}
          label={pick.asks === "url" ? "Server address (optional)" : "Server address"}
          name="baseUrl"
          type="url"
          inputMode="url"
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          placeholder={pick.urlHint}
          defaultValue={baseUrl}
          hint={addressHint(pick)}
          error={errors.baseUrl}
        />
      ) : null}
      {pick.asks !== "url" ? (
        <TextField
          key={`${pick.id}-key`}
          label={pick.asks === "both" ? "API key (optional)" : "API key"}
          name="apiKey"
          type="password"
          autoComplete="off"
          spellCheck={false}
          hint="Stored encrypted on your server. It is never shown again."
          error={errors.apiKey}
        />
      ) : null}
      {pick.api ? <ApiPicker key={`${pick.id}-api`} defaultValue={pick.api} /> : null}
    </>
  );
}

/** The add-provider fields of a submitted form, as `providerSchema` input. */
export function providerValues(data: FormData) {
  return {
    name: text(data, "name"),
    baseUrl: text(data, "baseUrl"),
    apiKey: text(data, "apiKey"),
    api: text(data, "api") || undefined,
  };
}
