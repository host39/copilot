import type {
  ChatResponse,
  MessageResponse,
  ModelResponse,
  ProjectResponse,
  ToolServerResponse,
  UpdateChatRequest,
} from "@copilot/contracts";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { fetchChat, fetchMessages, markRead, updateChat } from "@web/api/routes/chats";
import { deleteFile, uploadFile } from "@web/api/routes/files";
import { keys } from "@web/api/routes/keys";
import { fetchModels } from "@web/api/routes/models";
import { fetchProject } from "@web/api/routes/projects";
import { fetchToolServers } from "@web/api/routes/tool-servers";
import { Composer, type Picked, type Upload } from "@web/components/Chat/Composer";
import { type ListActions, MessageList } from "@web/components/Chat/MessageList";
import { VOICE_MESSAGE } from "@web/components/Chat/messages";
import { ToolsMenu } from "@web/components/Chat/ToolsMenu";
import { TOOLS_OFF_NOTE, toolsWarning } from "@web/components/Chat/tools";
import { FormAlert } from "@web/components/Form/FormAlert";
import { PageTitle } from "@web/components/Layout/PageTitle";
import { MenuButton, useOwnHeader } from "@web/components/Layout/Shell";
import { QueryState } from "@web/components/Settings/QueryState";
import { useSpeaker } from "@web/hooks/useSpeaker";
import { useChatSaved } from "@web/routes/chat/dialogs";
import { ChatHeader } from "@web/routes/chat/header";
import { contextUse, modelNotice, toolsOn } from "@web/routes/chat/settings";
import { type Run, timeline, type ViewMessage } from "@web/routes/chat/stream";
import { type Attached, type Phase, useTurn } from "@web/routes/chat/turn";
import path from "@web/routes/path";
import { scheduleContext } from "@web/routes/schedule/rules";
import { shortTokens } from "@web/routes/settings/provider/models";
import { type ReactNode, type RefObject, useEffect, useMemo, useRef, useState } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";

/** What the new chat screen hands over: the first message, sent once on arrival. */
export type ChatState = { text: string; files?: Attached[] };

/** The files of a sent message (attachments and recordings), to send again with an edit. */
export function messageFiles(message: Pick<MessageResponse, "parts">): Attached[] {
  return message.parts.flatMap((part) => {
    if (part.type === "file") {
      return [{ fileId: part.fileId, name: part.name, mediaType: part.mediaType }];
    }
    if (part.type === "audio") {
      const mediaType = part.format === "mp3" ? "audio/mpeg" : "audio/wav";
      return [{ fileId: part.fileId, name: VOICE_MESSAGE, mediaType }];
    }
    return [];
  });
}

/** Whether the chat's answers come with spoken audio: Speak replies on, for a model with `audioOut`. */
export function speaksReplies(
  chat: Pick<ChatResponse, "voiceReplies"> | undefined,
  model: Pick<ModelResponse, "capabilities"> | undefined
): boolean {
  return Boolean(chat?.voiceReplies && model?.capabilities.audioOut);
}

/** Sends the first message the new chat screen handed over, once. */
function useFirstMessage(
  first: ChatState | null,
  send: (text: string, files: Attached[]) => unknown
) {
  const location = useLocation();
  const navigate = useNavigate();
  const sent = useRef(false);
  useEffect(() => {
    if (!first || sent.current) return;
    sent.current = true;
    navigate(location.pathname, { replace: true, state: null });
    void send(first.text, first.files ?? []);
  }, [first, location.pathname, navigate, send]);
}

/** Opening the chat, and every answer seen in it (`seen` moves), clears its unread dot. */
function useMarkRead(chatId: string, seen: number) {
  const client = useQueryClient();
  useEffect(() => {
    if (!seen) return;
    markRead(chatId)
      .then(() => {
        // Only the lists: refreshing this chat's own queries would mark it read again.
        void client.invalidateQueries({ queryKey: [...keys.chats, "list"] });
        void client.invalidateQueries({ queryKey: keys.runs });
      })
      .catch(() => undefined);
  }, [chatId, client, seen]);
}

/**
 * Scrolls to the message a search result named (`hash`, once), else to the latest message
 * whenever `count` changes, and keeps the newest text in view while it streams, as long as the
 * user stays at the bottom.
 */
function useScroll(
  scroller: RefObject<HTMLDivElement | null>,
  count: number,
  progress: number,
  hash: string
) {
  const jumped = useRef("");
  const bottom = useRef(true);
  useEffect(() => {
    const element = scroller.current;
    if (!element) return;
    const track = () => {
      bottom.current = element.scrollHeight - element.scrollTop - element.clientHeight < 80;
    };
    element.addEventListener("scroll", track, { passive: true });
    return () => element.removeEventListener("scroll", track);
  }, [scroller]);
  useEffect(() => {
    if (count === 0) return;
    const target = hash && jumped.current !== hash ? document.getElementById(hash) : null;
    if (target) {
      jumped.current = hash;
      target.scrollIntoView({ block: "center" });
    } else if (!hash || jumped.current === hash) {
      scroller.current?.scrollTo({ top: scroller.current.scrollHeight });
    }
  }, [count, hash, scroller]);
  useEffect(() => {
    if (progress > 0 && bottom.current) {
      scroller.current?.scrollTo({ top: scroller.current.scrollHeight });
    }
  }, [progress, scroller]);
}

/** Esc stops the answer being written (desktop shortcut), unless a dialog or menu takes it. */
function useEscapeStop(running: boolean, stop: () => void) {
  useEffect(() => {
    if (!running) return;
    const listener = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      if (document.querySelector("dialog[open]")) return;
      stop();
    };
    document.addEventListener("keydown", listener);
    return () => document.removeEventListener("keydown", listener);
  }, [running, stop]);
}

/** The run's messages still being written. */
export function writingIds(run: Run | null, running: boolean): Set<string> {
  if (!running || !run) return new Set();
  return new Set(
    run.messages.filter((message) => message.status === "streaming").map((message) => message.id)
  );
}

/** What the conversation offers while no turn runs, through the turn's actions. */
function listActions(turn: ReturnType<typeof useTurn>): ListActions {
  return {
    edit: (message: ViewMessage, text: string) =>
      turn.send(text, messageFiles(message), message.id),
    regenerate: () => void turn.regenerate(),
    retry: () => void turn.retry(),
    continue: (text) => void turn.send(text),
    decide: turn.decide,
  };
}

/** A query's state as `QueryState` reads it. */
type Loading = { isPending: boolean; isError: boolean; error: unknown; refetch: () => unknown };

/**
 * The messages query as the conversation shows it: a spinner while the saved messages load, and
 * their error with a retry when none could be read. A turn that streams shows itself instead
 * (the new chat's first answer), and an error after messages were read stays above the composer.
 */
export function conversationQuery(
  messages: Loading & { data?: unknown },
  streaming: boolean
): Loading {
  const empty = messages.data === undefined && !streaming;
  return {
    isPending: messages.isPending && empty,
    isError: messages.isError && empty,
    error: messages.error,
    refetch: messages.refetch,
  };
}

/** What the line above the composer says about the connection to the run. */
export function phaseNote(phase: Phase): string | null {
  return phase === "reconnecting" ? "Connection lost. The answer goes on; reconnecting..." : null;
}

/**
 * Stores a change of the chat's settings at once: the screen shows it before the server answers,
 * the saved chat replaces it, and a refusal puts the chat back and reports the server's message
 * to `onError` (`null` when a change starts). Changes apply from the next message.
 */
function useChatChange(onError: (message: string | null) => void) {
  const client = useQueryClient();
  const saved = useChatSaved();
  return (chat: ChatResponse, body: UpdateChatRequest) => {
    onError(null);
    client.setQueryData(keys.chat(chat.id), { ...chat, ...body });
    updateChat(chat.id, body)
      .then((result) => saved(result))
      .catch((failure: unknown) => {
        onError(describeError(failure).message);
        void client.invalidateQueries({ queryKey: keys.chat(chat.id), exact: true });
      });
  };
}

/** The chat's tools menu; each change is stored on the chat at once (`useChatChange`). */
function ChatTools({
  chat,
  project,
  servers,
  model,
  onError,
}: {
  chat: ChatResponse;
  project: ProjectResponse | undefined;
  servers: ToolServerResponse[] | undefined;
  model: ModelResponse | undefined;
  /** The server's message when it refused a change, `null` when one starts. */
  onError: (message: string | null) => void;
}) {
  const change = useChatChange(onError);
  return (
    <ToolsMenu
      servers={servers}
      project={project}
      choice={{ toolOverrides: chat.toolOverrides, builtinTools: chat.builtinTools }}
      warning={toolsWarning(toolsOn(chat, project), model?.capabilities)}
      onChange={(choice) => change(chat, choice)}
    />
  );
}

/** The message field, with the turn's failure and what keeps the chat from answering. */
function Footer({
  alert,
  notice,
  phase,
  meter,
  model,
  chatId,
  send,
  stop,
  tools,
  toolsOff,
  changeError,
  speak,
}: {
  alert: string | null;
  notice: string | null;
  phase: Phase;
  meter: ReturnType<typeof contextUse>;
  model: ModelResponse | undefined;
  chatId: string;
  send: (text: string, files: Picked[]) => Promise<boolean>;
  stop: () => void;
  tools: ReactNode;
  /** Tools are on but the model cannot call them. */
  toolsOff: boolean;
  /** Why the server refused the last change of the chat's settings (tools, Speak replies). */
  changeError: string | null;
  speak: { on: boolean; onChange: (on: boolean) => void };
}) {
  const upload: Upload = (file, name, onProgress) =>
    uploadFile({ chatId }, file, name, {
      onProgress: (progress) => onProgress(progress.loaded),
    }).then((saved) => ({ id: saved.id, mediaType: saved.mediaType }));
  const note = phaseNote(phase);
  return (
    <div className="mx-auto w-full max-w-3xl space-y-2 px-4 pb-4 sm:px-8">
      <FormAlert>{alert ?? changeError}</FormAlert>
      {notice ? <p className="text-sm text-warn">{notice}</p> : null}
      {toolsOff ? <p className="text-sm text-warn">{TOOLS_OFF_NOTE}</p> : null}
      {note ? (
        <p role="status" className="text-sm text-muted">
          {note}
        </p>
      ) : null}
      {meter ? (
        <p className="text-right text-xs text-muted">
          {`Context: ${shortTokens(meter.used)} of ${shortTokens(meter.window)} (${meter.percent}%)`}
        </p>
      ) : null}
      <Composer
        onSend={send}
        onStop={stop}
        running={phase !== "idle"}
        disabled={notice !== null}
        capabilities={model?.capabilities}
        upload={upload}
        discard={(fileId) => void deleteFile(fileId).catch(() => undefined)}
        tools={tools}
        speak={speak}
      />
    </div>
  );
}

/**
 * One chat: the header (rename, model, actions), the conversation with the answer streaming in,
 * and the message field. A search result opens it at the matching message (`#message-<id>`).
 */
function ChatView({ chatId }: { chatId: string }) {
  useOwnHeader();
  const location = useLocation();
  const first = (location.state as ChatState | null) ?? null;
  const chat = useQuery({
    queryKey: keys.chat(chatId),
    queryFn: ({ signal }) => fetchChat(chatId, signal),
  });
  const messages = useQuery({
    queryKey: keys.chatMessages(chatId),
    queryFn: ({ signal }) => fetchMessages(chatId, signal),
  });
  const models = useQuery({ queryKey: keys.models, queryFn: ({ signal }) => fetchModels(signal) });
  const servers = useQuery({
    queryKey: keys.toolServers,
    queryFn: ({ signal }) => fetchToolServers(signal),
  });
  const projectId = chat.data?.projectId ?? null;
  const project = useQuery({
    queryKey: keys.project(projectId ?? ""),
    queryFn: ({ signal }) => fetchProject(projectId ?? "", signal),
    enabled: projectId !== null,
  });
  const data = chat.data;
  const model = models.data?.find((item) => item.id === data?.modelId);
  const speaker = useSpeaker(speaksReplies(data, model));
  const turn = useTurn(chatId, messages.data, first === null, speaker);
  const [changeError, setChangeError] = useState<string | null>(null);
  const change = useChatChange(setChangeError);
  const scroller = useRef<HTMLDivElement>(null);

  useFirstMessage(first, turn.send);
  useMarkRead(chatId, messages.dataUpdatedAt);
  const running = turn.phase !== "idle";
  useEscapeStop(running, turn.stop);

  const shown = timeline(messages.data ?? [], turn.run, turn.draft);
  useScroll(scroller, shown.length, turn.run?.seq ?? 0, location.hash.slice(1));

  const names = useMemo(
    () => new Map((servers.data ?? []).map((server) => [server.id, server.name])),
    [servers.data]
  );
  const actions = running ? undefined : listActions(turn);

  const notice = data ? modelNotice(data, models.data) : null;
  const conversation = conversationQuery(messages, turn.run !== null);
  // A failed load the conversation shows (with a retry) is not repeated above the composer.
  const problem = conversation.isError ? null : messages.error;

  return (
    <section className="flex h-full flex-col">
      <PageTitle page="Chat" />
      {data ? (
        <ChatHeader
          chat={data}
          project={project.data ?? null}
          models={models.data ?? []}
          toolsOn={toolsOn(data, project.data)}
        />
      ) : (
        <header className="flex min-h-14 items-center border-b border-line px-2 lg:hidden">
          <MenuButton />
        </header>
      )}
      <div ref={scroller} className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto w-full max-w-3xl px-4 py-6 sm:px-8">
          <QueryState query={chat} back={{ to: path.home, label: "New chat" }}>
            <QueryState query={conversation}>
              <MessageList
                messages={shown}
                writing={writingIds(turn.run, running)}
                running={running}
                windowStartId={turn.windowStartId}
                actions={actions}
                titles={turn.run?.titles}
                servers={names}
                schedule={scheduleContext(model, project.data, servers.data)}
              />
            </QueryState>
          </QueryState>
        </div>
      </div>
      {data ? (
        <Footer
          alert={turn.alert ?? (problem ? describeError(problem).message : null)}
          notice={notice}
          phase={turn.phase}
          meter={contextUse(shown, model)}
          model={model}
          chatId={chatId}
          send={(text, files) =>
            turn.send(
              text,
              files.flatMap((file) =>
                file.fileId
                  ? [{ fileId: file.fileId, name: file.name, mediaType: file.mediaType }]
                  : []
              )
            )
          }
          stop={() => void turn.stop()}
          tools={
            <ChatTools
              chat={data}
              project={project.data}
              servers={servers.data}
              model={model}
              onError={setChangeError}
            />
          }
          toolsOff={toolsWarning(toolsOn(data, project.data), model?.capabilities)}
          changeError={changeError}
          speak={{
            on: data.voiceReplies,
            onChange: (on) => {
              // The click lets the reply's audio start later; turning it off silences it.
              if (on) speaker.unlock();
              else speaker.stop();
              change(data, { voiceReplies: on });
            },
          }}
        />
      ) : null}
    </section>
  );
}

/** A chat; each chat gets its own view, so a run never follows the page to another chat. */
export default function ChatPage() {
  const { chatId = "" } = useParams();
  return <ChatView key={chatId} chatId={chatId} />;
}
