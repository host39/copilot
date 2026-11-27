import type { AuthMode } from "@copilot/contracts";

type AuthPickerProps = {
  value: AuthMode;
  onChange: (value: AuthMode) => void;
};

const OPTIONS: { value: AuthMode; label: string; hint: string }[] = [
  { value: "none", label: "None", hint: "The server needs no credentials." },
  {
    value: "headers",
    label: "Token or headers",
    hint: "A bearer token or custom headers you enter.",
  },
  {
    value: "oauth",
    label: "Sign in",
    hint: "OAuth: you sign in at the server's sign-in page.",
  },
];

/** How a tool server's requests authenticate. */
export function AuthPicker({ value, onChange }: AuthPickerProps) {
  return (
    <fieldset>
      <legend className="field-label">Authentication</legend>
      <div className="mt-2 grid gap-2 sm:grid-cols-3">
        {OPTIONS.map((option) => (
          <label key={option.value} className="choice-card flex-col gap-0.5">
            <input
              type="radio"
              name="authMode"
              value={option.value}
              checked={value === option.value}
              onChange={() => onChange(option.value)}
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
