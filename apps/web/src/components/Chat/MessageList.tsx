import type { MessagePartDto, MessageResponse } from "@copilot/contracts";
import type { Reply } from "@web/routes/chat/reply";

type MessageListProps = {
  messages: MessageResponse[];
  /** The user's message while its turn runs, before the saved messages are read again. */
  pending?: string | null;
  /** The answer streaming in. */
  reply?: Reply | null;
};

/** The text of a message's text parts. Other parts arrive with Phase 3. */
export function messageText(parts: MessagePartDto[]): string {
  return parts.flatMap((part) => (part.type === "text" ? [part.text] : [])).join("\n\n");
}

function Bubble({ own, children }: { own: boolean; children: string }) {
  return (
    <div className={own ? "flex justify-end" : "flex justify-start"}>
      <div
        className={`max-w-[85%] rounded-2xl px-4 py-2.5 text-sm leading-relaxed break-words whitespace-pre-wrap ${
          own ? "bg-accent text-on-accent" : "bg-raised text-fg"
        }`}
      >
        {children}
      </div>
    </div>
  );
}

/**
 * The conversation as plain text (no Markdown or HTML: model output is untrusted). Tool messages
 * are left out until Phase 3 draws tool cards.
 */
export function MessageList({ messages, pending, reply }: MessageListProps) {
  return (
    <ol className="space-y-3">
      {messages
        .filter((message) => message.role !== "tool")
        .map((message) => {
          const text = messageText(message.parts);
          return (
            <li key={message.id} className="space-y-1">
              {text ? <Bubble own={message.role === "user"}>{text}</Bubble> : null}
              {message.role === "assistant" && message.error ? (
                <p className="field-error">{message.error}</p>
              ) : null}
            </li>
          );
        })}
      {pending ? (
        <li>
          <Bubble own>{pending}</Bubble>
        </li>
      ) : null}
      {reply ? (
        <li className="space-y-1" aria-live="polite" aria-busy={!reply.done}>
          <Bubble own={false}>{reply.text || "..."}</Bubble>
          {reply.error ? <p className="field-error">{reply.error}</p> : null}
        </li>
      ) : null}
    </ol>
  );
}
