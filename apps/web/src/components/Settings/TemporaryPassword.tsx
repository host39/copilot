import Button from "@web/components/Button/Button";
import { TextField } from "@web/components/Form/TextField";
import { temporaryPassword } from "@web/routes/settings/user/accounts";
import { useState } from "react";
import { LuDices } from "react-icons/lu";

type TemporaryPasswordProps = {
  label: string;
  error?: string;
};

/**
 * A temporary password field the administrator fills in or generates. It shows as plain text, so
 * the password can be read out or copied before it is handed over.
 */
export function TemporaryPassword({ label, error }: TemporaryPasswordProps) {
  const [value, setValue] = useState("");
  return (
    <div className="space-y-2">
      <TextField
        label={label}
        name="password"
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        value={value}
        onChange={(event) => setValue(event.target.value)}
        hint="At least 12 characters. Hand it over in person or another safe way."
        error={error}
      />
      <Button variant="secondary" onClick={() => setValue(temporaryPassword())}>
        <LuDices aria-hidden />
        Generate
      </Button>
    </div>
  );
}
