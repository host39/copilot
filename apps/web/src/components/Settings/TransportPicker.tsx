import type { Transport } from "@copilot/contracts";

type TransportPickerProps = {
  value: Transport;
  onChange: (value: Transport) => void;
};

const OPTIONS: { value: Transport; label: string; hint: string }[] = [
  { value: "http", label: "Remote server", hint: "An MCP server at an http or https address." },
  {
    value: "stdio",
    label: "Program on this server",
    hint: "stdio: Copilot starts the program for each chat and talks to it over stdin and stdout.",
  },
];

/** Where a new tool server is: at an address, or a program Copilot starts (administrators). */
export function TransportPicker({ value, onChange }: TransportPickerProps) {
  return (
    <fieldset>
      <legend className="field-label">Kind</legend>
      <div className="mt-2 grid gap-2 sm:grid-cols-2">
        {OPTIONS.map((option) => (
          <label key={option.value} className="choice-card flex-col gap-0.5">
            <input
              type="radio"
              name="transport"
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
