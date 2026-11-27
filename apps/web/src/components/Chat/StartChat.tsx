import type { ProjectResponse, ReasoningLevel } from "@copilot/contracts";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { createChat, fetchChats } from "@web/api/routes/chats";
import { keys } from "@web/api/routes/keys";
import { fetchModels } from "@web/api/routes/models";
import { Composer } from "@web/components/Chat/Composer";
import { FormAlert } from "@web/components/Form/FormAlert";
import { ModelPicker } from "@web/components/ModelPicker/ModelPicker";
import { fitLevel, startModel } from "@web/components/ModelPicker/models";
import type { ChatState } from "@web/routes/chat/page";
import path, { link } from "@web/routes/path";
import { type ReactNode, useState } from "react";
import { Link, useNavigate } from "react-router-dom";

type StartChatProps = {
  /** The project the new chat goes into. */
  project?: ProjectResponse | null;
  /** A model to start with before the usual defaults (the first-run setup's). */
  preferred?: string;
  placeholder?: string;
  /** Shown beside the model picker (the new chat screen's project choice). */
  extra?: ReactNode;
};

/**
 * The new chat draft: a model (the project's default, else the model of the most recently updated
 * chat, else the first enabled one) and its reasoning level. The first send creates the chat with
 * the draft and opens it, which sends the message, so empty chats never exist.
 */
export function StartChat({ project, preferred, placeholder, extra }: StartChatProps) {
  const models = useQuery({ queryKey: keys.models, queryFn: ({ signal }) => fetchModels(signal) });
  const latest = useQuery({
    queryKey: keys.latestChat,
    queryFn: ({ signal }) => fetchChats({ limit: 1 }, signal),
  });
  const [choice, setChoice] = useState<{ modelId: string; reasoning: ReasoningLevel | null }>();
  const [alert, setAlert] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const navigate = useNavigate();
  const client = useQueryClient();

  const list = models.data ?? [];
  const model = startModel(
    list,
    choice?.modelId,
    preferred,
    project?.defaultModelId,
    latest.data?.items[0]?.modelId
  );
  const reasoning = fitLevel(model, choice?.reasoning ?? null);

  async function start(text: string): Promise<boolean> {
    if (!model) return false;
    setBusy(true);
    setAlert(null);
    try {
      const chat = await createChat({
        modelId: model.id,
        ...(project ? { projectId: project.id } : {}),
        ...(reasoning !== null ? { reasoning } : {}),
      });
      void client.invalidateQueries({ queryKey: keys.chats });
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

  if (models.isSuccess && list.length === 0) {
    return (
      <p className="text-sm text-muted">
        No models yet.{" "}
        <Link to={path.welcome} className="text-link">
          Add a provider
        </Link>{" "}
        or turn on models in{" "}
        <Link to={path.providers} className="text-link">
          Settings
        </Link>{" "}
        to start a chat.
      </p>
    );
  }

  return (
    <div className="space-y-2">
      <FormAlert>
        {alert ?? (models.isError ? describeError(models.error).message : null)}
      </FormAlert>
      <div className="flex flex-wrap items-center gap-2">
        <ModelPicker
          models={list}
          modelId={model?.id ?? null}
          reasoning={reasoning}
          disabled={!models.isSuccess}
          onChange={(next) => setChoice(next)}
        />
        {extra}
      </div>
      <Composer onSend={start} busy={busy} disabled={!model} placeholder={placeholder} />
    </div>
  );
}
