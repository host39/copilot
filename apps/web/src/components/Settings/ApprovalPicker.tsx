import type { ApprovalMode } from "@copilot/contracts";

type ApprovalPickerProps = {
  value: ApprovalMode;
  onChange: (value: ApprovalMode) => void;
};

const OPTIONS: { value: ApprovalMode; label: string; hint: string }[] = [
  { value: "ask", label: "Ask", hint: "Every tool call waits for your approval." },
  { value: "auto", label: "Auto", hint: "Tools run without asking." },
];

/**
 * A server's default approval mode. Ask is the default; Auto is an explicit choice that shows a
 * warning, because a tool's output can steer the model into calling other tools.
 */
export function ApprovalPicker({ value, onChange }: ApprovalPickerProps) {
  return (
    <fieldset>
      <legend className="field-label">Approval</legend>
      <div className="mt-2 grid gap-2 sm:grid-cols-2">
        {OPTIONS.map((option) => (
          <label key={option.value} className="choice-card flex-col gap-0.5">
            <input
              type="radio"
              name="approval"
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
      {value === "auto" ? (
        <p role="note" className="form-warning mt-2">
          Auto lets the model run this server's tools without asking. Use it only for servers you
          trust, whose tools cannot change or send your data. You can still set single tools to Ask.
        </p>
      ) : null}
    </fieldset>
  );
}
