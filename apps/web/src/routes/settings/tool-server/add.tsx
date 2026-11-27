import type { ApprovalMode } from "@copilot/contracts";
import { useQueryClient } from "@tanstack/react-query";
import { keys } from "@web/api/routes/keys";
import Button from "@web/components/Button/Button";
import { FormAlert } from "@web/components/Form/FormAlert";
import { TextField } from "@web/components/Form/TextField";
import { ApprovalPicker } from "@web/components/Settings/ApprovalPicker";
import { HeaderRows } from "@web/components/Settings/HeaderRows";
import { SettingsPage } from "@web/components/Settings/SettingsPage";
import { text } from "@web/forms";
import { useSubmit } from "@web/hooks/useSubmit";
import path, { link } from "@web/routes/path";
import {
  type AddedState,
  addServer,
  type HeaderRow,
  SLUG_MAX_CHARS,
  serverSchema,
} from "@web/routes/settings/tool-server/forms";
import { useState } from "react";
import { useNavigate } from "react-router-dom";

/** Settings, Tool servers, Add: registers the server, tests it, and opens its page. */
export function AddToolServer() {
  const client = useQueryClient();
  const navigate = useNavigate();
  const [approval, setApproval] = useState<ApprovalMode>("ask");
  const [rows, setRows] = useState<HeaderRow[]>([]);
  const form = useSubmit(serverSchema, addServer, (added) => {
    void client.invalidateQueries({ queryKey: keys.toolServers });
    const state: AddedState = { test: added.test, problem: added.problem };
    navigate(link.toolServer(added.server.id), { replace: true, state });
  });

  return (
    <SettingsPage
      title="Add Tool Server"
      back={{ to: path.toolServers, label: "Tool servers" }}
      description="A remote MCP server over Streamable HTTP. Copilot tests it by listing its tools."
    >
      <form
        noValidate
        className="card-panel space-y-6"
        onSubmit={(event) => {
          event.preventDefault();
          const data = new FormData(event.currentTarget);
          void form.submit({
            name: text(data, "name"),
            slug: text(data, "slug"),
            url: text(data, "url"),
            approval,
            bearer: text(data, "bearer"),
            rows,
          });
        }}
      >
        <FormAlert>{form.alert}</FormAlert>
        <TextField
          label="Name"
          name="name"
          autoComplete="off"
          placeholder="GitHub"
          error={form.errors.name}
        />
        <TextField
          label="Server address"
          name="url"
          type="url"
          inputMode="url"
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          placeholder="https://mcp.example.com/mcp"
          error={form.errors.url}
        />
        <TextField
          label="Bearer token (optional)"
          name="bearer"
          type="password"
          autoComplete="off"
          spellCheck={false}
          hint="Sent as Authorization: Bearer. Stored encrypted; it is never shown again."
          error={form.errors.bearer}
        />
        <HeaderRows rows={rows} onChange={setRows} error={form.errors.headers} />
        <ApprovalPicker value={approval} onChange={setApproval} />
        <TextField
          label="Short name (optional)"
          name="slug"
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          maxLength={SLUG_MAX_CHARS}
          hint={`Prefixes this server's tool names for the model. Up to ${SLUG_MAX_CHARS} lowercase letters or digits; taken from the name when empty.`}
          error={form.errors.slug}
        />
        <div className="flex justify-end">
          <Button type="submit" busy={form.busy} disabled={form.wait > 0}>
            {form.busy ? "Connecting" : "Add and test"}
          </Button>
        </div>
      </form>
    </SettingsPage>
  );
}
