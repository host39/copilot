import { useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { fetchChat, fetchMessages, sendMessage } from "@web/api/routes/chats";
import { Composer } from "@web/components/Chat/Composer";
import { MessageList } from "@web/components/Chat/MessageList";
import { FormAlert } from "@web/components/Form/FormAlert";
import { PageTitle } from "@web/components/Layout/PageTitle";
import { applyEvent, emptyReply, type Reply } from "@web/routes/chat/reply";
import { useCallback, useEffect, useRef, useState } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";

/** What the new chat screen hands over: the first message, sent once on arrival. */
export type ChatState = { text: string };

const chatKey = (chatId: string) => ["chats", chatId] as const;

/** One chat: its saved messages, the answer streaming in, and the message field. */
export default function ChatPage() {
  const { chatId = "" } = useParams();
  const location = useLocation();
  const navigate = useNavigate();
  const client = useQueryClient();
  const chat = useQuery({
    queryKey: chatKey(chatId),
    queryFn: ({ signal }) => fetchChat(chatId, signal),
  });
  const messages = useQuery({
    queryKey: [...chatKey(chatId), "messages"],
    queryFn: ({ signal }) => fetchMessages(chatId, signal),
  });
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
        await client.invalidateQueries({ queryKey: chatKey(chatId) });
        setPending(null);
        setReply(null);
      }
    },
    [chatId, client]
  );

  // The first message from the new chat screen, sent once.
  const first = (location.state as ChatState | null)?.text;
  const sent = useRef(false);
  useEffect(() => {
    if (!first || sent.current) return;
    sent.current = true;
    navigate(location.pathname, { replace: true, state: null });
    void send(first);
  }, [first, location.pathname, navigate, send]);

  const problem = chat.error ?? messages.error;

  return (
    <section className="mx-auto flex h-full w-full max-w-3xl flex-col gap-4 px-4 py-6 sm:px-8">
      <PageTitle page="Chat" />
      <h1 className="truncate text-lg font-semibold tracking-tight">
        {chat.data?.title || "New Chat"}
      </h1>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <MessageList messages={messages.data ?? []} pending={pending} reply={reply} />
      </div>
      <FormAlert>{alert ?? (problem ? describeError(problem).message : null)}</FormAlert>
      <Composer onSend={send} busy={reply !== null} disabled={chat.isError} />
    </section>
  );
}
