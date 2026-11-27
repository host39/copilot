import type {
  CreateChatRequest,
  ProjectResponse,
  ReasoningLevel,
  UpdateChatRequest,
} from "@copilot/contracts";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { createChat, deleteChat, fetchChats, updateChat } from "@web/api/routes/chats";
import { uploadFile } from "@web/api/routes/files";
import { keys } from "@web/api/routes/keys";
import { fetchModels } from "@web/api/routes/models";
import { fetchToolServers } from "@web/api/routes/tool-servers";
import { Composer, type Picked } from "@web/components/Chat/Composer";
import { draftKey, dropStart, keepStart, readStart } from "@web/components/Chat/drafts";
import { type ToolChoice, ToolsMenu } from "@web/components/Chat/ToolsMenu";
import { TOOLS_OFF_NOTE, toolsWarning } from "@web/components/Chat/tools";
import { FormAlert } from "@web/components/Form/FormAlert";
import { ModelPicker } from "@web/components/ModelPicker/ModelPicker";
import { fitLevel, startModel } from "@web/components/ModelPicker/models";
import type { ChatState } from "@web/routes/chat/page";
import { toolsOn } from "@web/routes/chat/settings";
import type { Attached } from "@web/routes/chat/turn";
import path, { link } from "@web/routes/path";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { Link, useNavigate } from "react-router-dom";

type StartChatProps = {
  /** The project the new chat goes into. */
  project?: ProjectResponse | null;
  /** A model to start with before the usual defaults (the first-run setup's). */
  preferred?: string;
  placeholder?: string;
  /** Shown beside the model picker (the new chat screen's project choice). */
  extra?: ReactNode;
  /**
   * Where the message being written is kept (`draftKey`); by default the project's new chat
   * draft. The new chat screen keeps one for every project choice, so the text follows a change.
   */
  draft?: string;
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
 * The `PATCH /chats/{id}` body that brings a chat created by a failed first send in line with the
 * draft as it is now: the model, reasoning, tools, and Speak replies may have changed since.
 */
export function retryBody(
  modelId: string,
  reasoning: ReasoningLevel | null,
  tools: ToolChoice,
  voiceReplies: boolean
): UpdateChatRequest {
  return {
    modelId,
    reasoning,
    toolOverrides: tools.toolOverrides,
    builtinTools: tools.builtinTools,
    voiceReplies,
  };
}

/**
 * The new chat draft: a model (the project's default, else the model of the most recently updated
 * chat, else the first enabled one), its reasoning level, and the tools (the project's, with the
 * draft's changes on top). The first send creates the chat with
 * the draft, uploads the attached files to it, and opens it, which sends the message, so empty
 * chats never exist. When an upload fails, sending again reuses the chat and the files already
 * uploaded, with the draft's current choices; leaving without a successful send deletes it.
 */
export function StartChat({ project, preferred, placeholder, extra, draft }: StartChatProps) {
  const models = useQuery({ queryKey: keys.models, queryFn: ({ signal }) => fetchModels(signal) });
  const latest = useQuery({
    queryKey: keys.latestChat,
    queryFn: ({ signal }) => fetchChats({ limit: 1 }, signal),
  });
  const servers = useQuery({
    queryKey: keys.toolServers,
    queryFn: ({ signal }) => fetchToolServers(signal),
  });
  const choices = draftKey({ projectId: project?.id ?? null });
  const [kept] = useState(() => readStart(choices));
  const [choice, setChoice] = useState(kept?.choice);
  const [tools, setTools] = useState<ToolChoice>(
    kept?.tools ?? { toolOverrides: [], builtinTools: null }
  );
  const [alert, setAlert] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [speak, setSpeak] = useState(kept?.speak ?? false);
  const navigate = useNavigate();
  const client = useQueryClient();
  // The chat and uploads of a first send that failed part way, reused when sending again.
  const created = useRef<{ chatId: string; files: Map<string, Attached> } | null>(null);

  // The choices are kept with the message (`Composer`'s draft), so "Add tool server..." or a look
  // at another page never resets them.
  useEffect(() => {
    if (choice || speak || tools.toolOverrides.length > 0 || tools.builtinTools !== null) {
      keepStart(choices, { choice, tools, speak });
    } else dropStart(choices);
  }, [choices, choice, tools, speak]);

  // A chat a failed first send created holds no message: it goes when the draft is left.
  useEffect(
    () => () => {
      const chat = created.current;
      if (!chat) return;
      created.current = null;
      deleteChat(chat.chatId)
        .then(() => client.invalidateQueries({ queryKey: keys.chats }))
        .catch(() => undefined);
    },
    [client]
  );

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
      } else {
        await updateChat(
          created.current.chatId,
          retryBody(model.id, reasoning, tools, voiceReplies)
        );
      }
      const { chatId, files } = created.current;
      const state: ChatState = { text, files: await uploadAll(chatId, picked, files) };
      // The chat now has its first message on the way: it stays, and the draft is done.
      created.current = null;
      dropStart(choices);
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
        draft={draft ?? choices}
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
