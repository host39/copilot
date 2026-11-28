import { TextField } from "@web/components/Form/TextField";

/**
 * A pre-registered OAuth client: only for authorization servers that offer Copilot no other way
 * to get a client id.
 */
export function ClientFields({
  error,
  defaultId,
  secretStored = false,
}: {
  error?: string;
  defaultId?: string;
  secretStored?: boolean;
}) {
  return (
    <div className="space-y-4">
      <TextField
        label="Client ID (optional)"
        name="clientId"
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        defaultValue={defaultId}
        hint="Only when the server's sign-in needs a client you registered there yourself, with the redirect URI Copilot names (here, after the first Sign in). Copilot registers itself when the server allows it."
        error={error}
      />
      <TextField
        label="Client secret (optional)"
        name="clientSecret"
        type="password"
        autoComplete="off"
        spellCheck={false}
        hint={
          secretStored
            ? "A secret is stored. Leave this empty to keep it."
            : "For a confidential client. Stored encrypted; it is never shown again."
        }
      />
    </div>
  );
}
