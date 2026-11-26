import { useQuery } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { createChat } from "@web/api/routes/chats";
import { fetchModels } from "@web/api/routes/models";
import { Composer } from "@web/components/Chat/Composer";
import { FormAlert } from "@web/components/Form/FormAlert";
import { PageTitle } from "@web/components/Layout/PageTitle";
import type { ChatState } from "@web/routes/chat/page";
import path, { link } from "@web/routes/path";
import { useState } from "react";
import { Link, useNavigate } from "react-router-dom";

/** New chat: the default screen. Pick a model and send the first message. */
export default function HomePage() {
  const models = useQuery({
    queryKey: ["models"],
    queryFn: ({ signal }) => fetchModels(signal),
  });
  const [chosen, setChosen] = useState("");
  const [alert, setAlert] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const navigate = useNavigate();
  const list = models.data ?? [];
  const modelId = list.some((model) => model.id === chosen) ? chosen : (list[0]?.id ?? "");

  async function start(text: string): Promise<boolean> {
    setBusy(true);
    setAlert(null);
    try {
      const chat = await createChat({ modelId });
      const state: ChatState = { text };
      navigate(link.chat(chat.id), { state });
      return true;
    } catch (error) {
      setAlert(describeError(error).message);
      return false;
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="mx-auto flex h-full w-full max-w-3xl flex-col gap-6 px-4 py-8 sm:px-8">
      <PageTitle page="New Chat" />
      <h1 className="text-xl font-semibold tracking-tight">New Chat</h1>
      <FormAlert>
        {alert ?? (models.isError ? describeError(models.error).message : null)}
      </FormAlert>
      {models.isSuccess && list.length === 0 ? (
        <p className="text-sm text-muted">
          No models yet. Add a provider connection in{" "}
          <Link to={path.providers} className="text-link">
            Settings
          </Link>{" "}
          to start a chat.
        </p>
      ) : (
        <div className="mt-auto space-y-3">
          <div>
            <label className="field-label" htmlFor="model">
              Model
            </label>
            <select
              id="model"
              className="field-input"
              value={modelId}
              disabled={!models.isSuccess}
              onChange={(event) => setChosen(event.target.value)}
            >
              {list.map((model) => (
                <option key={model.id} value={model.id}>
                  {`${model.providerName} / ${model.displayName}`}
                </option>
              ))}
            </select>
          </div>
          <Composer onSend={start} busy={busy} disabled={modelId === ""} />
        </div>
      )}
    </section>
  );
}
