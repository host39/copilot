import type { CallDecision, ToolStatus } from "@copilot/contracts";
import { Badge, type BadgeTone } from "@web/components/Badge/Badge";
import Button from "@web/components/Button/Button";
import { FileChip } from "@web/components/Chat/FileChip";
import { choose, readyDecisions } from "@web/components/Chat/messages";
import { ScheduleProposal } from "@web/components/Schedule/Proposal";
import type { ToolCall, ToolResult } from "@web/routes/chat/stream";
import { readProposal, type ScheduleContext } from "@web/routes/schedule/rules";
import { useId, useState } from "react";
import { LuCheck, LuCheckCheck, LuX } from "react-icons/lu";

/**
 * How the cards name tools: titles from the stream, server names from the settings, and what a
 * schedule the assistant asks to create would run with.
 */
export type ToolNames = {
  /** Display titles by call id. */
  titles?: Record<string, string>;
  /** Tool server names by id. */
  servers?: ReadonlyMap<string, string>;
  /** The chat's model, project, and `auto` tools, for the `schedule_prompt` card. */
  schedule?: ScheduleContext;
};

const STATUS_MAP: Record<ToolStatus, { label: string; tone: BadgeTone }> = {
  ok: { label: "Done", tone: "ok" },
  error: { label: "Failed", tone: "danger" },
  denied: { label: "Denied", tone: "warn" },
  cancelled: { label: "Cancelled", tone: "neutral" },
  timeout: { label: "Timed out", tone: "danger" },
};

/**
 * Where a tool call stands: its result's status, else its approval, else running (while the run
 * goes on) or without a result.
 */
export function callStatus(call: ToolCall, result: ToolResult | undefined, running = false) {
  if (result) return STATUS_MAP[result.status];
  if (call.approval === "pending") return { label: "Needs approval", tone: "warn" as const };
  if (call.approval === "denied") return STATUS_MAP.denied;
  if (running) return { label: "Running", tone: "neutral" as const };
  return { label: "No result", tone: "neutral" as const };
}

/** A tool result's text; images show as files instead. */
export function resultText(result: ToolResult): string {
  return result.content.flatMap((item) => (item.type === "text" ? [item.text] : [])).join("\n");
}

/** Arguments as the cards show them: parsed JSON indented, raw text as it came. */
export function argumentText(value: unknown): string {
  return typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

/** The tool's title and where it comes from ("Weather forecast", "from Weather"). */
export function toolLabel(call: ToolCall, names: ToolNames): { title: string; source: string } {
  const title = names.titles?.[call.callId] || call.toolName;
  if (call.serverId === null) return { title, source: "Built-in" };
  return { title, source: names.servers?.get(call.serverId) ?? "Tool server" };
}

function Arguments({ call }: { call: ToolCall }) {
  return (
    <>
      <p className="text-xs text-muted">Arguments</p>
      <pre className="max-h-64 overflow-auto text-xs whitespace-pre-wrap">
        {argumentText(call.arguments)}
      </pre>
    </>
  );
}

type ToolCardProps = ToolNames & {
  call: ToolCall;
  result: ToolResult | undefined;
  /** The run that made the call is still going. */
  running?: boolean;
};

/** A tool call: title, server, and status, with the arguments and result folded away. */
export function ToolCard({ call, result, running = false, ...names }: ToolCardProps) {
  const status = callStatus(call, result, running);
  const { title, source } = toolLabel(call, names);
  const text = result ? resultText(result) : "";
  const images = result?.content.flatMap((item) => (item.type === "image" ? [item] : [])) ?? [];
  return (
    <details className="rounded-xl border border-line bg-surface text-sm">
      <summary className="flex min-h-10 cursor-pointer items-center gap-2 px-3 py-2">
        <span className="min-w-0 flex-1 truncate">
          <span className="font-medium">{title}</span>
          <span className="ml-2 text-xs text-muted">{source}</span>
        </span>
        <Badge tone={status.tone}>{status.label}</Badge>
      </summary>
      <div className="space-y-2 border-t border-line px-3 py-2">
        <Arguments call={call} />
        {call.denyReason ? (
          <p className="text-xs text-muted">{`Reason: ${call.denyReason}`}</p>
        ) : null}
        {result ? (
          <>
            <p className="text-xs text-muted">Result</p>
            {text ? (
              <pre className="max-h-64 overflow-auto text-xs whitespace-pre-wrap">{text}</pre>
            ) : null}
            {images.map((image, index) => (
              <FileChip key={image.fileId} fileId={image.fileId} name={`Image ${index + 1}`} />
            ))}
            {result.truncated ? (
              <p className="text-xs text-muted">The result is shortened here.</p>
            ) : null}
          </>
        ) : null}
      </div>
    </details>
  );
}

type Choice = "once" | "chat" | "deny";

const CHOICE_LABEL: Record<Choice, string> = {
  once: "Approved once",
  chat: "Approved for this chat",
  deny: "Denied",
};

function ApprovalCard({
  call,
  choice,
  busy,
  onChoose,
  names,
}: {
  call: ToolCall;
  choice: Choice | undefined;
  busy: boolean;
  onChoose: (choice: Choice, reason: string) => void;
  names: ToolNames;
}) {
  const [reason, setReason] = useState("");
  const id = useId();
  const { title, source } = toolLabel(call, names);
  const proposal = readProposal(call);
  return (
    <li className="space-y-2 rounded-xl border border-warn/40 bg-surface px-3 py-3 text-sm">
      <p>
        <span className="font-medium">{title}</span>
        <span className="ml-2 text-xs text-muted">{source}</span>
      </p>
      {proposal ? (
        <ScheduleProposal proposal={proposal} context={names.schedule} />
      ) : (
        <>
          <p className="text-muted">The model wants to run this tool.</p>
          <Arguments call={call} />
        </>
      )}
      {choice ? (
        <p className="flex items-center gap-2">
          <Badge tone={choice === "deny" ? "warn" : "ok"}>{CHOICE_LABEL[choice]}</Badge>
          {busy ? null : (
            <button
              type="button"
              className="text-link text-xs"
              onClick={() => onChoose(choice, "")}
            >
              Change
            </button>
          )}
        </p>
      ) : (
        <>
          <label htmlFor={id} className="field-label">
            Reason for denying (optional)
          </label>
          <input
            id={id}
            value={reason}
            onChange={(event) => setReason(event.target.value)}
            maxLength={500}
            className="field-input mt-0"
          />
          <div className="flex flex-wrap gap-2">
            <Button disabled={busy} onClick={() => onChoose("once", "")}>
              <LuCheck aria-hidden />
              Approve
            </Button>
            {call.serverId !== null ? (
              <Button variant="secondary" disabled={busy} onClick={() => onChoose("chat", "")}>
                <LuCheckCheck aria-hidden />
                Approve for this chat
              </Button>
            ) : null}
            <Button variant="secondary" disabled={busy} onClick={() => onChoose("deny", reason)}>
              <LuX aria-hidden />
              Deny
            </Button>
          </div>
        </>
      )}
    </li>
  );
}

type ApprovalPanelProps = ToolNames & {
  calls: ToolCall[];
  /** Sends the decisions; resolves `false` when the server refused them. */
  onDecide: (decisions: CallDecision[]) => Promise<boolean>;
};

/**
 * The approval cards of an answer that waits: Approve, Approve for this chat, or Deny with an
 * optional reason, per call. The decisions go together once every call has one (the server takes
 * them only together); a choice can change until then.
 */
export function ApprovalPanel({ calls, onDecide, ...names }: ApprovalPanelProps) {
  const [choices, setChoices] = useState<
    Record<string, { choice: Choice; decision: CallDecision }>
  >({});
  const [busy, setBusy] = useState(false);

  async function pick(call: ToolCall, choice: Choice, reason: string) {
    const current = choices[call.callId];
    // Choosing a call's choice again takes it back ("Change").
    const next = { ...choices };
    if (current?.choice === choice && reason === "") delete next[call.callId];
    else next[call.callId] = { choice, decision: choose(call, choice, reason) };
    setChoices(next);
    const decisions = readyDecisions(
      calls,
      Object.fromEntries(Object.entries(next).map(([key, value]) => [key, value.decision]))
    );
    if (!decisions) return;
    setBusy(true);
    const ok = await onDecide(decisions);
    setBusy(false);
    if (!ok) setChoices({});
  }

  const decided = calls.filter((call) => choices[call.callId]).length;
  return (
    <section aria-label="Tool approvals" className="space-y-2">
      <ul className="space-y-2">
        {calls.map((call) => (
          <ApprovalCard
            key={call.callId}
            call={call}
            choice={choices[call.callId]?.choice}
            busy={busy}
            names={names}
            onChoose={(choice, reason) => void pick(call, choice, reason)}
          />
        ))}
      </ul>
      {calls.length > 1 ? (
        <p className="text-xs text-muted">{`${decided} of ${calls.length} decided. The answer continues once every call is decided.`}</p>
      ) : null}
    </section>
  );
}
