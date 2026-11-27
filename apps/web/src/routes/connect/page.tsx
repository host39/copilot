import { fetchHealth } from "@web/api/routes/health";
import { AuthCard } from "@web/components/Auth/AuthCard";
import { SubmitButton } from "@web/components/Auth/SubmitButton";
import { FormAlert } from "@web/components/Form/FormAlert";
import { TextField } from "@web/components/Form/TextField";
import { needsHttpWarning, parseServerUrl, saveServerUrl, serverUrl } from "@web/config";
import { text } from "@web/forms";
import path from "@web/routes/path";
import { useState } from "react";
import { LuTriangleAlert } from "react-icons/lu";
import { Link, useNavigate } from "react-router-dom";

/**
 * Native builds: the address of the Copilot server, checked with `GET /health` before it is
 * stored. Plain `http` to a public address needs explicit consent.
 */
export default function ConnectPage() {
  const navigate = useNavigate();
  const [error, setError] = useState<string>();
  const [alert, setAlert] = useState<string>();
  const [insecure, setInsecure] = useState<string>();
  const [busy, setBusy] = useState(false);
  const current = serverUrl();

  async function connect(data: FormData): Promise<void> {
    setError(undefined);
    setAlert(undefined);
    const url = parseServerUrl(text(data, "url"));
    if (url === undefined) {
      setError("Enter a full address that starts with https:// or http://.");
      return;
    }
    if (needsHttpWarning(url) && data.get("insecure") !== "on") {
      setInsecure(url);
      return;
    }
    setBusy(true);
    try {
      await fetchHealth(url);
    } catch {
      setAlert("No Copilot server answered at this address.");
      setBusy(false);
      return;
    }
    saveServerUrl(url);
    navigate(path.login, { replace: true });
  }

  return (
    <AuthCard
      title="Connect to Server"
      server={false}
      footer={
        current === undefined ? null : (
          <Link to={path.home} className="text-link">
            Keep the current server
          </Link>
        )
      }
      description="Enter the address of your Copilot server, as you would open it in a browser."
    >
      <form
        noValidate
        className="space-y-4"
        onSubmit={(event) => {
          event.preventDefault();
          void connect(new FormData(event.currentTarget));
        }}
      >
        <FormAlert>{alert}</FormAlert>
        <TextField
          label="Server address"
          name="url"
          type="url"
          inputMode="url"
          autoComplete="url"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          placeholder="https://copilot.example.com"
          defaultValue={current ?? ""}
          onChange={() => setInsecure(undefined)}
          required
          error={error}
        />
        {insecure ? (
          <div className="rounded-lg border border-warn/40 bg-warn/10 p-3 text-sm">
            <p className="flex gap-2 font-medium text-warn">
              <LuTriangleAlert aria-hidden className="mt-0.5 shrink-0" />
              This address is not encrypted.
            </p>
            <p className="mt-1 text-muted">
              Your password and session would cross the network in the clear. Use https:// unless
              you trust every network between you and the server.
            </p>
            <label className="mt-3 flex items-center gap-2">
              <input type="checkbox" name="insecure" className="size-4" />
              Connect without encryption
            </label>
          </div>
        ) : null}
        <SubmitButton busy={busy} wait={0}>
          Connect
        </SubmitButton>
      </form>
    </AuthCard>
  );
}
