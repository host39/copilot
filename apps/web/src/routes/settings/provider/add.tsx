import { useQueryClient } from "@tanstack/react-query";
import { keys } from "@web/api/routes/keys";
import Button from "@web/components/Button/Button";
import { FormAlert } from "@web/components/Form/FormAlert";
import { ProviderFields, providerValues } from "@web/components/Settings/ProviderFields";
import { SettingsPage } from "@web/components/Settings/SettingsPage";
import { useSubmit } from "@web/hooks/useSubmit";
import path, { link } from "@web/routes/path";
import {
  CHOICES,
  type Choice,
  type ConnectState,
  connect,
  providerSchema,
} from "@web/routes/settings/providers/presets";
import { useState } from "react";
import { useNavigate } from "react-router-dom";

/** Settings, Providers, Add: saves the connection, tests it, and opens its page with the result. */
export function AddProvider() {
  const [pick, setPick] = useState<Choice>(CHOICES[0] as Choice);
  const client = useQueryClient();
  const navigate = useNavigate();
  const form = useSubmit(providerSchema(pick), connect, (connected) => {
    void client.invalidateQueries({ queryKey: keys.providers });
    void client.invalidateQueries({ queryKey: keys.models });
    const state: ConnectState = { refresh: connected.refresh, problem: connected.problem };
    navigate(link.provider(connected.provider.id), { replace: true, state });
  });

  return (
    <SettingsPage
      title="Add Provider"
      back={{ to: path.providers, label: "Providers" }}
      description="Copilot tests the connection by listing its models."
    >
      <form
        noValidate
        className="card-panel space-y-6"
        onSubmit={(event) => {
          event.preventDefault();
          void form.submit(providerValues(new FormData(event.currentTarget)));
        }}
      >
        <FormAlert>{form.alert}</FormAlert>
        <ProviderFields pick={pick} onPick={setPick} errors={form.errors} />
        <div className="flex justify-end">
          <Button type="submit" busy={form.busy} disabled={form.wait > 0}>
            {form.busy ? "Connecting" : "Add and test"}
          </Button>
        </div>
      </form>
    </SettingsPage>
  );
}
