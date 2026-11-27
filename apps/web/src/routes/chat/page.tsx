import type { MessageResponse, ModelResponse } from "@copilot/contracts";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { fetchChat, fetchMessages, markRead } from "@web/api/routes/chats";
import { deleteFile, uploadFile } from "@web/api/routes/files";
import { keys } from "@web/api/routes/keys";
import { fetchModels } from "@web/api/routes/models";
import { fetchProject } from "@web/api/routes/projects";
import { fetchToolServers } from "@web/api/routes/tool-servers";
import { Composer, type Picked, type Upload } from "@web/components/Chat/Composer";
import { type ListActions, MessageList } from "@web/components/Chat/MessageList";
import { FormAlert } from "@web/components/Form/FormAlert";
import { PageTitle } from "@web/components/Layout/PageTitle";
import { MenuButton, useOwnHeader } from "@web/components/Layout/Shell";
import { QueryState } from "@web/components/Settings/QueryState";
import { ChatHeader } from "@web/routes/chat/header";
import { contextUse, modelNotice, toolsOn } from "@web/routes/chat/settings";
import { type Run, timeline, type ViewMessage } from "@web/routes/chat/stream";
import { type Attached, type Phase, useTurn } from "@web/routes/chat/turn";
import path from "@web/routes/path";
import { shortTokens } from "@web/routes/settings/provider/models";
import { type RefObject, useEffect, useMemo, useRef } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";

/** What the new chat screen hands over: the first message, sent once on arrival. */
export type ChatState = { text: string; files?: Attached[] };

/** The files of a sent message, to send again with an edit. */
export function messageFiles(message: Pick<MessageResponse, "parts">): Attached[] {
  return message.parts.flatMap((part) =>
    part.type === "file"
      ? [{ fileId: part.fileId, name: part.name, mediaType: part.mediaType }]
      : []
  );
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

/** What the line above the composer says about the connection to the run. */
export function phaseNote(phase: Phase): string | null {
  return phase === "reconnecting" ? "Connection lost. The answer goes on; reconnecting..." : null;
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
}: {
  alert: string | null;
  notice: string | null;
  phase: Phase;
  meter: ReturnType<typeof contextUse>;
  model: ModelResponse | undefined;
  chatId: string;
  send: (text: string, files: Picked[]) => Promise<boolean>;
  stop: () => void;
}) {
  const upload: Upload = (file, name, onProgress) =>
    uploadFile({ chatId }, file, name, {
      onProgress: (progress) => onProgress(progress.loaded),
    }).then((saved) => ({ id: saved.id, mediaType: saved.mediaType }));
  const note = phaseNote(phase);
  return (
    <div className="mx-auto w-full max-w-3xl space-y-2 px-4 pb-4 sm:px-8">
      <FormAlert>{alert}</FormAlert>
      {notice ? <p className="text-sm text-warn">{notice}</p> : null}
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
  const turn = useTurn(chatId, messages.data, first === null);
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

  const data = chat.data;
  const model = models.data?.find((item) => item.id === data?.modelId);
  const notice = data ? modelNotice(data, models.data) : null;
  const problem = messages.error;

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
            <MessageList
              messages={shown}
              writing={writingIds(turn.run, running)}
              running={running}
              windowStartId={turn.windowStartId}
              actions={actions}
              titles={turn.run?.titles}
              servers={names}
            />
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
