import type { ProviderResponse } from "@copilot/contracts";
import { useQuery } from "@tanstack/react-query";
import { keys } from "@web/api/routes/keys";
import { fetchProviders } from "@web/api/routes/providers";
import { Badge } from "@web/components/Badge/Badge";
import { QueryState } from "@web/components/Settings/QueryState";
import { SettingsPage } from "@web/components/Settings/SettingsPage";
import path, { link } from "@web/routes/path";
import { choice } from "@web/routes/settings/providers/presets";
import { LuChevronRight, LuPlus } from "react-icons/lu";
import { Link } from "react-router-dom";

/** The address without its scheme, for a compact row ("api.openai.com/v1"). */
function address(url: string): string {
  return url.replace(/^https?:\/\//, "");
}

function Row({ provider }: { provider: ProviderResponse }) {
  const preset = choice(provider.kind, provider.preset).label;
  return (
    <li>
      <Link to={link.provider(provider.id)} className="list-row">
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-center gap-2 text-sm font-medium">
            <span className="break-words">{provider.name}</span>
            {provider.enabled ? null : <Badge>Off</Badge>}
            {provider.key === "unreadable" ? <Badge tone="danger">Key unreadable</Badge> : null}
          </p>
          <p className="truncate text-sm text-muted">
            {`${preset === provider.name ? "" : `${preset} · `}${address(provider.baseUrl)}`}
          </p>
        </div>
        <LuChevronRight aria-hidden className="shrink-0 text-muted" />
      </Link>
    </li>
  );
}

/** Settings, Providers: the caller's connections to model providers. */
export default function ProvidersPage() {
  const providers = useQuery({
    queryKey: keys.providers,
    queryFn: ({ signal }) => fetchProviders(signal),
  });
  const list = providers.data ?? [];

  return (
    <SettingsPage
      title="Providers"
      description="Connections to model providers. Chats go to the models you enable here."
      actions={
        <Link to={path.providerNew} className="button-link">
          <LuPlus aria-hidden />
          Add provider
        </Link>
      }
    >
      <QueryState query={providers}>
        {list.length === 0 ? (
          <div className="card-panel text-center">
            <p className="text-sm font-medium">No providers yet</p>
            <p className="mt-1 text-sm text-muted">
              Add OpenAI, Anthropic, Gemini, OpenRouter, Ollama, or any compatible server.
            </p>
          </div>
        ) : (
          <ul className="list-panel">
            {list.map((provider) => (
              <Row key={provider.id} provider={provider} />
            ))}
          </ul>
        )}
      </QueryState>
    </SettingsPage>
  );
}
