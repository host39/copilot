import { useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { fetchChat, fetchMessages, markRead, sendMessage } from "@web/api/routes/chats";
import { keys } from "@web/api/routes/keys";
import { fetchModels } from "@web/api/routes/models";
import { fetchProject } from "@web/api/routes/projects";
import { Composer } from "@web/components/Chat/Composer";
import { MessageList } from "@web/components/Chat/MessageList";
import { FormAlert } from "@web/components/Form/FormAlert";
import { PageTitle } from "@web/components/Layout/PageTitle";
import { MenuButton, useOwnHeader } from "@web/components/Layout/Shell";
import { QueryState } from "@web/components/Settings/QueryState";
import { ChatHeader } from "@web/routes/chat/header";
import { applyEvent, emptyReply, type Reply } from "@web/routes/chat/reply";
import { contextUse, modelNotice, toolsOn } from "@web/routes/chat/settings";
import path from "@web/routes/path";
import { shortTokens } from "@web/routes/settings/provider/models";
import { type RefObject, useCallback, useEffect, useRef, useState } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";

/** What the new chat screen hands over: the first message, sent once on arrival. */
export type ChatState = { text: string };

/** A turn of the chat: sends the message and builds the answer from its events as they stream. */
function useTurn(chatId: string) {
  const client = useQueryClient();
  const [pending, setPending] = useState<string | null>(null);
  const [reply, setReply] = useState<Reply | null>(null);
  const [alert, setAlert] = useState<string | null>(null);

  const send = useCallback(
    async (text: string): Promise<boolean> => {
      setAlert(null);
      setPending(text);
      setReply(emptyReply);
      try {
        await sendMessage(chatId, { text, fileIds: [] }, (event) =>
          setReply((current) => current && applyEvent(current, event))
        );
        return true;
      } catch (error) {
        setAlert(describeError(error).message);
        return false;
      } finally {
        // The saved messages replace what streamed.
        await client.invalidateQueries({ queryKey: keys.chat(chatId) });
        setPending(null);
        setReply(null);
      }
    },
    [chatId, client]
  );

  return { send, pending, reply, alert };
}

/** Sends the first message the new chat screen handed over, once. */
function useFirstMessage(send: (text: string) => Promise<boolean>) {
  const location = useLocation();
  const navigate = useNavigate();
  const first = (location.state as ChatState | null)?.text;
  const sent = useRef(false);
  useEffect(() => {
    if (!first || sent.current) return;
    sent.current = true;
    navigate(location.pathname, { replace: true, state: null });
    void send(first);
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
 * whenever `count` changes.
 */
function useScrollTarget(scroller: RefObject<HTMLDivElement | null>, count: number, hash: string) {
  const jumped = useRef("");
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
}

/** The message field, with the turn's failure and what keeps the chat from answering. */
function Footer({
  alert,
  notice,
  meter,
  send,
  busy,
}: {
  alert: string | null;
  notice: string | null;
  meter: ReturnType<typeof contextUse>;
  send: (text: string) => Promise<boolean>;
  busy: boolean;
}) {
  return (
    <div className="mx-auto w-full max-w-3xl space-y-2 px-4 pb-4 sm:px-8">
      <FormAlert>{alert}</FormAlert>
      {notice ? <p className="text-sm text-warn">{notice}</p> : null}
      {meter ? (
        <p className="text-right text-xs text-muted">
          {`Context: ${shortTokens(meter.used)} of ${shortTokens(meter.window)} (${meter.percent}%)`}
        </p>
      ) : null}
      <Composer onSend={send} busy={busy} disabled={notice !== null} />
    </div>
  );
}

/**
 * One chat: the header (rename, model, actions), its saved messages, the answer streaming in, and
 * the message field. A search result opens it at the matching message (`#message-<id>`).
 */
export default function ChatPage() {
  useOwnHeader();
  const { chatId = "" } = useParams();
  const location = useLocation();
  const chat = useQuery({
    queryKey: keys.chat(chatId),
    queryFn: ({ signal }) => fetchChat(chatId, signal),
  });
  const messages = useQuery({
    queryKey: keys.chatMessages(chatId),
    queryFn: ({ signal }) => fetchMessages(chatId, signal),
  });
  const models = useQuery({ queryKey: keys.models, queryFn: ({ signal }) => fetchModels(signal) });
  const projectId = chat.data?.projectId ?? null;
  const project = useQuery({
    queryKey: keys.project(projectId ?? ""),
    queryFn: ({ signal }) => fetchProject(projectId ?? "", signal),
    enabled: projectId !== null,
  });
  const { send, pending, reply, alert } = useTurn(chatId);
  const scroller = useRef<HTMLDivElement>(null);

  useFirstMessage(send);
  useMarkRead(chatId, messages.dataUpdatedAt);

  const count = (messages.data?.length ?? 0) + (pending ? 1 : 0) + (reply ? 1 : 0);
  useScrollTarget(scroller, count, location.hash.slice(1));

  const data = chat.data;
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
            <MessageList messages={messages.data ?? []} pending={pending} reply={reply} />
          </QueryState>
        </div>
      </div>
      {data ? (
        <Footer
          alert={alert ?? (problem ? describeError(problem).message : null)}
          notice={notice}
          meter={contextUse(
            messages.data ?? [],
            models.data?.find((item) => item.id === data.modelId)
          )}
          send={send}
          busy={reply !== null}
        />
      ) : null}
    </section>
  );
}
