import type { CallDecision } from "@copilot/contracts";
import Button from "@web/components/Button/Button";
import { FileChip } from "@web/components/Chat/FileChip";
import { Markdown } from "@web/components/Chat/Markdown";
import {
  CONTINUE_TEXT,
  continueReason,
  editWarning,
  keyRejected,
  lastAnswer,
  laterCount,
  messageText,
} from "@web/components/Chat/messages";
import { ApprovalPanel, ToolCard, type ToolNames } from "@web/components/Chat/ToolCard";
import { ConfirmDialog } from "@web/components/Dialog/ConfirmDialog";
import { toolResults } from "@web/routes/chat/settings";
import { DRAFT_ID, pendingCalls, type ToolResult, type ViewMessage } from "@web/routes/chat/stream";
import path from "@web/routes/path";
import { messageAnchor } from "@web/routes/search/highlight";
import { memo, type ReactNode, useId, useState } from "react";
import { LuCheck, LuCopy, LuPencil, LuRefreshCw, LuRotateCcw } from "react-icons/lu";
import { Link } from "react-router-dom";

export { messageText } from "@web/components/Chat/messages";

/** What the user can do with the conversation while no turn runs. */
export type ListActions = {
  /** Edits a user message and sends it again; `false` when the server refused. */
  edit: (message: ViewMessage, text: string) => Promise<boolean>;
  regenerate: () => void;
  retry: () => void;
  /** Sends "Continue." after an answer that stopped at a limit. */
  continue: (text: string) => void;
  decide: (messageId: string, decisions: CallDecision[]) => Promise<boolean>;
};

type MessageListProps = ToolNames & {
  messages: ViewMessage[];
  /** Messages a run is writing now. */
  writing?: ReadonlySet<string>;
  /** A run is going: tool calls without a result are running. */
  running?: boolean;
  /** The first message the model saw; earlier ones are marked as not sent. */
  windowStartId?: string | null;
  /** `undefined` while a turn runs. */
  actions?: ListActions;
};

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

/** A small action under a message (copy, edit, regenerate). */
function Action({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      title={label}
      className="flex size-8 items-center justify-center rounded-md text-muted hover:bg-raised hover:text-fg focus-visible:outline-2 focus-visible:outline-accent"
    >
      {children}
    </button>
  );
}

function CopyAction({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <Action
      label={copied ? "Copied" : "Copy"}
      onClick={() => {
        void navigator.clipboard?.writeText(text).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1_500);
        });
      }}
    >
      {copied ? <LuCheck aria-hidden /> : <LuCopy aria-hidden />}
    </Action>
  );
}

/** One answer's Markdown; earlier parts keep their rendering while a later one streams. */
const Text = memo(function Text({ text }: { text: string }) {
  return <Markdown>{text}</Markdown>;
});

function Reasoning({ text, writing }: { text: string; writing: boolean }) {
  return (
    <details className="text-sm text-muted">
      <summary className="cursor-pointer">{writing ? "Thinking..." : "Reasoning"}</summary>
      <p className="mt-1 border-l-2 border-line pl-3 whitespace-pre-wrap">{text}</p>
    </details>
  );
}

function Assistant({
  message,
  results,
  writing,
  running,
  names,
}: {
  message: ViewMessage;
  results: Map<string, ToolResult>;
  writing: boolean;
  running: boolean;
  names: ToolNames;
}) {
  const visible = message.parts.some((part) => part.type !== "text" || part.text);
  return (
    // Screen readers hear an answer once it is written, not every piece of it.
    <div className="space-y-2" aria-live="polite" aria-busy={writing || undefined}>
      {message.parts.map((part, index) => {
        const key = `${message.id}-${index}`;
        switch (part.type) {
          case "text":
            return part.text ? <Text key={key} text={part.text} /> : null;
          case "reasoning":
            return part.text ? (
              <Reasoning
                key={key}
                text={part.text}
                writing={writing && index === message.parts.length - 1}
              />
            ) : null;
          case "toolCall":
            return (
              <ToolCard
                key={key}
                call={part}
                result={results.get(part.callId)}
                running={running}
                {...names}
              />
            );
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
      {writing && !visible ? (
        <p className="text-sm text-muted">
          <span className="animate-pulse">Answering...</span>
        </p>
      ) : null}
    </div>
  );
}

function UserFiles({ message }: { message: ViewMessage }) {
  const files = message.parts.flatMap((part) =>
    part.type === "file" || part.type === "audio" ? [part] : []
  );
  if (files.length === 0) return null;
  return (
    <div className="flex flex-wrap justify-end gap-2">
      {files.map((part) => (
        <FileChip
          key={part.fileId}
          fileId={part.fileId}
          name={part.type === "file" ? part.name : "Voice message"}
        />
      ))}
    </div>
  );
}

/** A user message: its files and text, and Edit (send again, replacing everything after it). */
function User({
  message,
  later,
  onEdit,
}: {
  message: ViewMessage;
  /** How many later messages an edit deletes. */
  later: number;
  onEdit?: (text: string) => Promise<boolean>;
}) {
  const text = messageText(message.parts);
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(text);
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const id = useId();
  const warning = editWarning(later);
  const hasFiles = message.parts.some((part) => part.type !== "text");

  async function submit() {
    if (!onEdit) return;
    setConfirming(false);
    setBusy(true);
    const ok = await onEdit(value.trim());
    setBusy(false);
    if (ok) setEditing(false);
  }

  if (editing && onEdit) {
    const empty = value.trim() === "" && !hasFiles;
    return (
      <form
        className="space-y-2"
        onSubmit={(event) => {
          event.preventDefault();
          if (empty) return;
          if (warning) setConfirming(true);
          else void submit();
        }}
      >
        <UserFiles message={message} />
        <label htmlFor={id} className="sr-only">
          Edit message
        </label>
        <textarea
          id={id}
          value={value}
          rows={3}
          // biome-ignore lint/a11y/noAutofocus: the field opens because the user asked to edit
          autoFocus
          onChange={(event) => setValue(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              setEditing(false);
            }
          }}
          className="field-input mt-0 max-h-72 min-h-20 resize-y"
        />
        {warning ? <p className="field-hint mt-0">{warning}</p> : null}
        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={() => setEditing(false)} disabled={busy}>
            Cancel
          </Button>
          <Button type="submit" busy={busy} disabled={empty}>
            Send
          </Button>
        </div>
        <ConfirmDialog
          open={confirming}
          title="Edit and Resend?"
          confirm="Edit and resend"
          busy={busy}
          onConfirm={() => void submit()}
          onCancel={() => setConfirming(false)}
        >
          <p>{warning}</p>
        </ConfirmDialog>
      </form>
    );
  }

  return (
    <div className="group space-y-1">
      <UserFiles message={message} />
      {text ? <Bubble own>{text}</Bubble> : null}
      {onEdit ? (
        <div className="flex justify-end">
          <Action
            label="Edit"
            onClick={() => {
              setValue(text);
              setEditing(true);
            }}
          >
            <LuPencil aria-hidden />
          </Action>
        </div>
      ) : null}
    </div>
  );
}

/** A failed answer: the error, a link to the provider settings for a rejected key, and Retry. */
function ErrorBanner({ error, onRetry }: { error: string; onRetry?: () => void }) {
  return (
    <div role="alert" className="form-alert flex flex-wrap items-center gap-x-3 gap-y-2">
      <span className="min-w-0 flex-1">{error}</span>
      {keyRejected(error) ? (
        <Link to={path.providers} className="text-link">
          Provider settings
        </Link>
      ) : null}
      {onRetry ? (
        <Button variant="secondary" onClick={onRetry}>
          <LuRotateCcw aria-hidden />
          Retry
        </Button>
      ) : null}
    </div>
  );
}

/** An answer cut short by a limit, with Continue. */
function LimitNote({ reason, onContinue }: { reason: string; onContinue?: () => void }) {
  return (
    <div className="flex flex-wrap items-center gap-3 text-sm text-muted">
      <span>{reason}</span>
      {onContinue ? (
        <Button variant="secondary" onClick={onContinue}>
          Continue
        </Button>
      ) : null}
    </div>
  );
}

/**
 * How an answer ended, under it: stopped, failed (with Retry), declined, waiting for decisions
 * elsewhere, or cut short by a limit (with Continue). Retry and Continue belong to the last answer.
 */
function Ending({ message, actions }: { message: ViewMessage; actions?: ListActions }) {
  switch (message.status) {
    case "cancelled":
      return <p className="text-sm text-muted">Stopped.</p>;
    case "error":
      return <ErrorBanner error={message.error ?? "The answer failed."} onRetry={actions?.retry} />;
    case "refused":
      return message.error ? <p className="form-warning">{message.error}</p> : null;
    case "awaiting_approval":
      return actions ? null : <p className="text-sm text-warn">Waiting for a tool decision.</p>;
    default: {
      const reason = continueReason(message);
      if (!reason) return null;
      return (
        <LimitNote
          reason={reason}
          onContinue={actions ? () => actions.continue(CONTINUE_TEXT) : undefined}
        />
      );
    }
  }
}

type RowProps = {
  message: ViewMessage;
  /** Every message on screen, for what an edit deletes. */
  messages: ViewMessage[];
  results: Map<string, ToolResult>;
  names: ToolNames;
  /** The message is the last answer: Retry, Continue, decisions, Copy, and Regenerate are its. */
  last: boolean;
  writing: boolean;
  running: boolean;
  /** Earlier messages were left out of the model's context. */
  divider: boolean;
  actions?: ListActions;
};

/** An answer with its approvals, its ending, and (the last one) Copy and Regenerate. */
function Answer({
  message,
  results,
  names,
  writing,
  running,
  own,
}: Omit<RowProps, "messages" | "last" | "divider" | "actions"> & {
  /** The actions, when this is the last answer and no turn runs. */
  own?: ListActions;
}) {
  const pending = own ? pendingCalls(message) : [];
  const text = messageText(message.parts);
  return (
    <>
      <Assistant
        message={message}
        results={results}
        writing={writing}
        running={running}
        names={names}
      />
      {own && pending.length > 0 ? (
        <ApprovalPanel
          calls={pending}
          onDecide={(decisions) => own.decide(message.id, decisions)}
          {...names}
        />
      ) : null}
      <Ending message={message} actions={own} />
      {own && pending.length === 0 ? (
        <div className="flex gap-1">
          {text ? <CopyAction text={text} /> : null}
          <Action label="Regenerate" onClick={own.regenerate}>
            <LuRefreshCw aria-hidden />
          </Action>
        </div>
      ) : null}
    </>
  );
}

function Row({ message, messages, last, divider, actions, ...rest }: RowProps) {
  const draft = message.id === DRAFT_ID;
  return (
    <li id={draft ? undefined : messageAnchor(message.id)} className="scroll-mt-4 space-y-2">
      {divider ? (
        <p className="flex items-center gap-3 text-xs text-muted">
          <span aria-hidden className="h-px flex-1 bg-line" />
          Earlier messages not sent
          <span aria-hidden className="h-px flex-1 bg-line" />
        </p>
      ) : null}
      {message.role === "user" ? (
        <User
          message={message}
          later={laterCount(messages, message.id)}
          onEdit={actions && !draft ? (value) => actions.edit(message, value) : undefined}
        />
      ) : (
        <Answer message={message} own={last ? actions : undefined} {...rest} />
      )}
    </li>
  );
}

/**
 * The conversation: saved messages, the user's new message, and the answer streaming in. User
 * text shows as typed; answers render as Markdown without raw HTML (model output is untrusted),
 * with reasoning folded away, tool calls as cards with their results (tool messages show inside
 * the calls they answer), approval cards, error banners with Retry, and Continue after a limit.
 * Each user message offers Edit and the last answer Copy and Regenerate while no turn runs.
 */
export function MessageList({
  messages,
  writing,
  running = false,
  windowStartId = null,
  actions,
  titles,
  servers,
}: MessageListProps) {
  const results = toolResults(messages);
  const last = lastAnswer(messages);
  const names: ToolNames = { titles, servers };
  const shown = messages.filter((message) => message.role !== "tool");
  return (
    <ol className="space-y-4">
      {shown.map((message, index) => (
        <Row
          key={message.id}
          message={message}
          messages={messages}
          results={results}
          names={names}
          last={message.id === last?.id}
          writing={writing?.has(message.id) ?? false}
          running={running}
          divider={message.id === windowStartId && index > 0}
          actions={actions}
        />
      ))}
    </ol>
  );
}
