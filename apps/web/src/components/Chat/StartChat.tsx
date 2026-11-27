import type { CreateChatRequest, ProjectResponse, ReasoningLevel } from "@copilot/contracts";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { createChat, fetchChats } from "@web/api/routes/chats";
import { uploadFile } from "@web/api/routes/files";
import { keys } from "@web/api/routes/keys";
import { fetchModels } from "@web/api/routes/models";
import { fetchToolServers } from "@web/api/routes/tool-servers";
import { Composer, type Picked } from "@web/components/Chat/Composer";
import { type ToolChoice, ToolsMenu } from "@web/components/Chat/ToolsMenu";
import { TOOLS_OFF_NOTE, toolsWarning } from "@web/components/Chat/tools";
import { FormAlert } from "@web/components/Form/FormAlert";
import { ModelPicker } from "@web/components/ModelPicker/ModelPicker";
import { fitLevel, startModel } from "@web/components/ModelPicker/models";
import type { ChatState } from "@web/routes/chat/page";
import { toolsOn } from "@web/routes/chat/settings";
import type { Attached } from "@web/routes/chat/turn";
import path, { link } from "@web/routes/path";
import { type ReactNode, useRef, useState } from "react";
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
 * Uploads the picked files to the new chat in order, skipping those `done` already holds (a send
 * that failed part way), and records each one there.
 */
async function uploadAll(
  chatId: string,
  picked: Picked[],
  done: Map<string, Attached>
): Promise<Attached[]> {
  const sent: Attached[] = [];
  for (const item of picked) {
    let file = done.get(item.key);
    if (!file) {
      const saved = await uploadFile({ chatId }, item.file, item.name);
      file = { fileId: saved.id, name: saved.name, mediaType: saved.mediaType };
      done.set(item.key, file);
    }
    sent.push(file);
  }
  return sent;
}

/**
 * The `POST /chats` body of the draft: only what differs from the defaults, so a chat in a project
 * keeps following the project's tools unless the draft changed them. `voiceReplies` is Speak
 * replies, sent only when on.
 */
export function draftBody(
  modelId: string,
  project: Pick<ProjectResponse, "id"> | null | undefined,
  reasoning: ReasoningLevel | null,
  tools: ToolChoice,
  voiceReplies = false
): CreateChatRequest {
  return {
    modelId,
    ...(project ? { projectId: project.id } : {}),
    ...(reasoning !== null ? { reasoning } : {}),
    ...(tools.toolOverrides.length > 0 ? { toolOverrides: tools.toolOverrides } : {}),
    ...(tools.builtinTools !== null ? { builtinTools: tools.builtinTools } : {}),
    ...(voiceReplies ? { voiceReplies: true } : {}),
  };
}

/**
 * The new chat draft: a model (the project's default, else the model of the most recently updated
 * chat, else the first enabled one), its reasoning level, and the tools (the project's, with the
 * draft's changes on top). The first send creates the chat with
 * the draft, uploads the attached files to it, and opens it, which sends the message, so empty
 * chats never exist. When an upload fails, sending again reuses the chat and the files already
 * uploaded.
 */
export function StartChat({ project, preferred, placeholder, extra }: StartChatProps) {
  const models = useQuery({ queryKey: keys.models, queryFn: ({ signal }) => fetchModels(signal) });
  const latest = useQuery({
    queryKey: keys.latestChat,
    queryFn: ({ signal }) => fetchChats({ limit: 1 }, signal),
  });
  const servers = useQuery({
    queryKey: keys.toolServers,
    queryFn: ({ signal }) => fetchToolServers(signal),
  });
  const [choice, setChoice] = useState<{ modelId: string; reasoning: ReasoningLevel | null }>();
  const [tools, setTools] = useState<ToolChoice>({ toolOverrides: [], builtinTools: null });
  const [alert, setAlert] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [speak, setSpeak] = useState(false);
  const navigate = useNavigate();
  const client = useQueryClient();
  // The chat and uploads of a first send that failed part way, reused when sending again.
  const created = useRef<{ chatId: string; files: Map<string, Attached> } | null>(null);

  const list = models.data ?? [];
  const model = startModel(
    list,
    choice?.modelId,
    preferred,
    project?.defaultModelId,
    latest.data?.items[0]?.modelId
  );
  const reasoning = fitLevel(model, choice?.reasoning ?? null);
  const toolsOff = toolsWarning(toolsOn(tools, project), model?.capabilities);
  // Speak replies counts only for a model that answers with audio.
  const voiceReplies = speak && Boolean(model?.capabilities.audioOut);

  async function start(text: string, picked: Picked[]): Promise<boolean> {
    if (!model) return false;
    setBusy(true);
    setAlert(null);
    try {
      if (!created.current) {
        const chat = await createChat(draftBody(model.id, project, reasoning, tools, voiceReplies));
        created.current = { chatId: chat.id, files: new Map() };
        void client.invalidateQueries({ queryKey: keys.chats });
      }
      const { chatId, files } = created.current;
      const state: ChatState = { text, files: await uploadAll(chatId, picked, files) };
      navigate(link.chat(chatId), { state });
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
      {toolsOff ? <p className="text-sm text-warn">{TOOLS_OFF_NOTE}</p> : null}
      <Composer
        onSend={start}
        busy={busy}
        disabled={!model}
        placeholder={placeholder}
        capabilities={model?.capabilities}
        speak={{ on: speak, onChange: setSpeak }}
        tools={
          <ToolsMenu
            servers={servers.data}
            project={project}
            choice={tools}
            onChange={setTools}
            warning={toolsOff}
          />
        }
      />
    </div>
  );
}
