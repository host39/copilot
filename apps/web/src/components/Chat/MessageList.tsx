import type { MessagePartDto, MessageResponse, ToolStatus } from "@copilot/contracts";
import { Badge, type BadgeTone } from "@web/components/Badge/Badge";
import { FileChip } from "@web/components/Chat/FileChip";
import { Markdown } from "@web/components/Chat/Markdown";
import type { Reply } from "@web/routes/chat/reply";
import { toolResults } from "@web/routes/chat/settings";
import { messageAnchor } from "@web/routes/search/highlight";
import type { ReactNode } from "react";

type MessageListProps = {
  messages: MessageResponse[];
  /** The user's message while its turn runs, before the saved messages are read again. */
  pending?: string | null;
  /** The answer streaming in. */
  reply?: Reply | null;
};

type ToolCall = Extract<MessagePartDto, { type: "toolCall" }>;
type ToolResult = Extract<MessagePartDto, { type: "toolResult" }>;

/** The text of a message's text parts. */
export function messageText(parts: MessagePartDto[]): string {
  return parts.flatMap((part) => (part.type === "text" ? [part.text] : [])).join("\n\n");
}

const STATUS_MAP: Record<ToolStatus, { label: string; tone: BadgeTone }> = {
  ok: { label: "Done", tone: "ok" },
  error: { label: "Failed", tone: "danger" },
  denied: { label: "Denied", tone: "warn" },
  cancelled: { label: "Cancelled", tone: "neutral" },
  timeout: { label: "Timed out", tone: "danger" },
};

/** Where a tool call stands: its result's status, or its approval while there is none. */
export function callStatus(call: ToolCall, result: ToolResult | undefined) {
  if (result) return STATUS_MAP[result.status];
  if (call.approval === "pending") return { label: "Needs approval", tone: "warn" as const };
  if (call.approval === "denied") return STATUS_MAP.denied;
  return { label: "No result", tone: "neutral" as const };
}

/** A tool result's content as plain text; images are named, not shown. */
export function resultText(result: ToolResult): string {
  return result.content.map((item) => (item.type === "text" ? item.text : "[image]")).join("\n");
}

function json(value: unknown): string {
  return typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

function Bubble({ own, children }: { own: boolean; children: ReactNode }) {
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

/** A saved tool call: its name and status, with the arguments and result as preformatted text. */
function CallCard({ call, result }: { call: ToolCall; result: ToolResult | undefined }) {
  const status = callStatus(call, result);
  return (
    <details className="rounded-xl border border-line bg-surface text-sm">
      <summary className="flex min-h-10 cursor-pointer items-center gap-2 px-3 py-2">
        <span className="min-w-0 flex-1 truncate font-medium">{call.toolName}</span>
        <Badge tone={status.tone}>{status.label}</Badge>
      </summary>
      <div className="space-y-2 border-t border-line px-3 py-2">
        <p className="text-xs text-muted">Arguments</p>
        <pre className="overflow-x-auto text-xs whitespace-pre-wrap">{json(call.arguments)}</pre>
        {call.denyReason ? (
          <p className="text-xs text-muted">{`Reason: ${call.denyReason}`}</p>
        ) : null}
        {result ? (
          <>
            <p className="text-xs text-muted">Result</p>
            <pre className="max-h-64 overflow-auto text-xs whitespace-pre-wrap">
              {resultText(result)}
            </pre>
            {result.truncated ? (
              <p className="text-xs text-muted">The result was cut to the size limit.</p>
            ) : null}
          </>
        ) : null}
      </div>
    </details>
  );
}

function Assistant({
  message,
  results,
}: {
  message: MessageResponse;
  results: Map<string, ToolResult>;
}) {
  return (
    <div className="space-y-2">
      {message.parts.map((part, index) => {
        const key = `${message.id}-${index}`;
        switch (part.type) {
          case "text":
            return part.text ? <Markdown key={key}>{part.text}</Markdown> : null;
          case "reasoning":
            return part.text ? (
              <details key={key} className="text-sm text-muted">
                <summary className="cursor-pointer">Reasoning</summary>
                <p className="mt-1 border-l-2 border-line pl-3 whitespace-pre-wrap">{part.text}</p>
              </details>
            ) : null;
          case "toolCall":
            return <CallCard key={key} call={part} result={results.get(part.callId)} />;
          case "file":
            return <FileChip key={key} fileId={part.fileId} name={part.name} />;
          case "audio":
            return (
              <div key={key} className="space-y-1">
                <FileChip fileId={part.fileId} name="Audio reply" />
                {part.transcript ? <p className="text-sm">{part.transcript}</p> : null}
              </div>
            );
          default:
            return null;
        }
      })}
    </div>
  );
}

function User({ message }: { message: MessageResponse }) {
  const text = messageText(message.parts);
  const files = message.parts.flatMap((part) =>
    part.type === "file" || part.type === "audio" ? [part] : []
  );
  return (
    <div className="space-y-1">
      {files.length > 0 ? (
        <div className="flex flex-wrap justify-end gap-2">
          {files.map((part) => (
            <FileChip
              key={part.fileId}
              fileId={part.fileId}
              name={part.type === "file" ? part.name : "Voice message"}
            />
          ))}
        </div>
      ) : null}
      {text ? <Bubble own>{text}</Bubble> : null}
    </div>
  );
}

/**
 * The conversation. User text shows as typed; answers render as Markdown without raw HTML
 * (model output is untrusted), with reasoning folded away, saved tool calls with their results,
 * and files to download. Tool messages show inside the call they answer.
 */
export function MessageList({ messages, pending, reply }: MessageListProps) {
  const results = toolResults(messages);
  return (
    <ol className="space-y-4">
      {messages
        .filter((message) => message.role !== "tool")
        .map((message) => (
          <li key={message.id} id={messageAnchor(message.id)} className="scroll-mt-4 space-y-1">
            {message.role === "user" ? (
              <User message={message} />
            ) : (
              <Assistant message={message} results={results} />
            )}
            {message.role === "assistant" && message.status === "cancelled" ? (
              <p className="text-sm text-muted">Stopped.</p>
            ) : null}
            {message.role === "assistant" && message.status === "awaiting_approval" ? (
              <p className="text-sm text-warn">Waiting for a tool decision.</p>
            ) : null}
            {message.role === "assistant" && message.error ? (
              <p role="alert" className="form-alert">
                {message.error}
              </p>
            ) : null}
          </li>
        ))}
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
