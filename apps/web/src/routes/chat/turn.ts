// A chat's turns on the chat screen: send (and edit and resend), regenerate, retry, decisions, and
// stop, each followed through its event stream. The page attaches to a running turn when it
// opens, and again after a dropped connection (mobile networks drop), asking for the events after
// the last `seq` it applied. Generation never depends on the page: the server keeps answering
// while nobody reads, so leaving the page only stops reading.

import type { CallDecision, ChatStreamEvent, MessageResponse } from "@copilot/contracts";
import { useQueryClient } from "@tanstack/react-query";
import { ApiError, isAbort, NETWORK_ERROR } from "@web/api/client";
import { describeError } from "@web/api/errors";
import { attach, decide, regenerate, retry, sendMessage, stop } from "@web/api/routes/chats";
import { keys } from "@web/api/routes/keys";
import { log } from "@web/logging";
import {
  DRAFT_ID,
  type Draft,
  decided,
  type Run,
  receive,
  type ViewMessage,
} from "@web/routes/chat/stream";
import { useCallback, useEffect, useRef, useState } from "react";

/** A file sent with a message. */
export type Attached = { fileId: string; name: string; mediaType: string };

/** Where the page stands with the chat's run. */
export type Phase = "idle" | "streaming" | "reconnecting";

/** How reading a stream ended. */
type Outcome =
  | { kind: "done" }
  /** The server had no run (`204`). */
  | { kind: "idle" }
  /** The connection dropped before `done`; the run goes on. */
  | { kind: "dropped"; events: number }
  | { kind: "failed"; error: unknown; events: number }
  /** The page stopped reading (it closed, or another stream replaced this one). */
  | { kind: "aborted" }
  /** A reattached stream belongs to another run: attach again from the start. */
  | { kind: "restart" };

type Open = (
  onEvent: (event: ChatStreamEvent) => void,
  signal: AbortSignal
) => Promise<boolean | undefined>;

/** The wait before reconnect `attempt` (from 0): 1, 2, 4, 8, then every 15 seconds. */
export function backoff(attempt: number): number {
  return Math.min(1_000 * 2 ** Math.max(0, attempt), 15_000);
}

/**
 * Whether a stream failed because the connection dropped (the run goes on without the page)
 * rather than because the server refused: a request that never reached it, or a body that
 * stopped mid-stream.
 */
export function dropped(error: unknown): boolean {
  if (isAbort(error)) return false;
  if (error instanceof ApiError) return error.code === NETWORK_ERROR;
  return true;
}

/** How a stream that threw ended: replaced, stopped by the page, dropped, or refused. */
function failure(
  error: unknown,
  stream: { restart: boolean; aborted: boolean; events: number }
): Outcome {
  if (stream.restart) return { kind: "restart" };
  if (stream.aborted || isAbort(error)) return { kind: "aborted" };
  if (dropped(error)) return { kind: "dropped", events: stream.events };
  return { kind: "failed", error, events: stream.events };
}

/** The user's new message as the screen shows it until it is saved. */
export function draftMessage(text: string, files: Attached[]): ViewMessage {
  return {
    id: DRAFT_ID,
    role: "user",
    status: "complete",
    finishReason: null,
    usage: null,
    error: null,
    parts: [
      ...(text ? [{ type: "text" as const, text }] : []),
      ...files.map((file) => ({ type: "file" as const, ...file })),
    ],
  };
}

/** The first message regenerating replaces: the one after the last user message. */
export function regenerateCut(messages: MessageResponse[]): string | null {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (messages[index]?.role === "user") return messages[index + 1]?.id ?? null;
  }
  return null;
}

/** The failed answer a retry replaces (with the results after it), when the last answer failed. */
export function retryCut(messages: Pick<MessageResponse, "id" | "role" | "status">[]) {
  const last = [...messages].reverse().find((message) => message.role === "assistant");
  return last?.status === "error" ? last.id : null;
}

/**
 * The chat's run as the page follows it: the run's messages, the user's new message until it is
 * saved, the context window's start, and the actions that start or stop a turn. `saved` is the
 * chat's saved messages; `attachOnOpen` is `false` when the page sends a message as it opens.
 */
export function useTurn(chatId: string, saved: MessageResponse[] | undefined, attachOnOpen = true) {
  const client = useQueryClient();
  const [run, setRun] = useState<Run | null>(null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [phase, setPhase] = useState<Phase>("idle");
  const [alert, setAlert] = useState<string | null>(null);
  const [windowStartId, setWindowStart] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const current = useRef<Run | null>(null);
  const connection = useRef<AbortController | null>(null);
  const mounted = useRef(true);
  const savedRef = useRef(saved);
  savedRef.current = saved;

  const show = useCallback((next: Run | null) => {
    current.current = next;
    setRun(next);
  }, []);

  /**
   * Reads one stream into the run. A turn the user starts, and an attach from the start, begin a
   * fresh run; an attach after `after` continues the run the page holds (see `resume`).
   */
  const follow = useCallback(
    async (open: Open, after: number | null): Promise<Outcome> => {
      connection.current?.abort();
      const controller = new AbortController();
      connection.current = controller;
      let events = 0;
      let ended = false;
      let restart = false;
      const onEvent = (event: ChatStreamEvent) => {
        if (controller.signal.aborted) return;
        const next = receive(current.current, event, after, events === 0);
        if (!next) {
          restart = true;
          controller.abort();
          return;
        }
        events += 1;
        if (event.type === "message.started") setWindowStart(event.windowStartId);
        ended ||= event.type === "done";
        show(next);
      };
      try {
        const streamed = await open(onEvent, controller.signal);
        if (streamed === false) return { kind: "idle" };
        return ended ? { kind: "done" } : { kind: "dropped", events };
      } catch (error) {
        return failure(error, { restart, aborted: controller.signal.aborted, events });
      } finally {
        if (connection.current === controller) connection.current = null;
      }
    },
    [show]
  );

  /** The run is over: read the saved messages, which replace the run's, then let it go. */
  const settle = useCallback(async () => {
    await client.invalidateQueries({ queryKey: keys.chat(chatId) });
    const ended = current.current;
    // A failure without a failed answer (nothing was saved) stays on screen.
    if (ended?.error && !ended.messages.some((message) => message.status === "error")) {
      setAlert(ended.error.message);
    }
    show(null);
    setDraft(null);
    setPhase("idle");
    setAttempt(0);
  }, [chatId, client, show]);

  /** Done following without a run to finish: the page is idle again. */
  const release = useCallback(async () => {
    if (current.current) await settle();
    else setPhase("idle");
  }, [settle]);

  const reattach = useCallback(
    async (after: number | null): Promise<void> => {
      if (!mounted.current) return;
      const outcome = await follow(
        (onEvent, signal) =>
          attach(chatId, onEvent, { after: after === null ? undefined : after, signal }),
        after
      );
      switch (outcome.kind) {
        case "restart":
          return reattach(null);
        case "dropped":
          // Without a run on screen there is nothing to catch up with: going online or coming
          // back to the page looks again.
          if (!current.current) return release();
          setPhase("reconnecting");
          setAttempt((value) => value + 1);
          return;
        case "failed":
          log("warn", "web.chat.attach.failed", {
            chatId,
            status: outcome.error instanceof ApiError ? outcome.error.status : undefined,
          });
          return release();
        case "aborted":
          return;
        case "idle":
          // Nothing runs: a run the page held ended while it was away.
          return release();
        case "done":
          return settle();
      }
    },
    [chatId, follow, release, settle]
  );

  /** A turn's stream dropped: follow the run again; `false` when nothing arrived (keep the text). */
  const lost = useCallback((events: number) => {
    setPhase("reconnecting");
    setAttempt(1);
    if (events > 0) return true;
    // The request may never have arrived: keep the text, and look for a run anyway.
    setAlert("The server cannot be reached.");
    setDraft(null);
    return false;
  }, []);

  /** The server refused a turn (`false`: the text stays), or failed it part way. */
  const refused = useCallback(
    async (error: unknown, events: number) => {
      setAlert(describeError(error).message);
      if (events > 0) {
        await settle();
        return true;
      }
      setDraft(null);
      setPhase("idle");
      return false;
    },
    [settle]
  );

  /** Starts a turn and follows it. `false` when the server refused it (the text stays for another try). */
  const start = useCallback(
    async (open: Open, next: Draft | null): Promise<boolean> => {
      setAlert(null);
      setDraft(next);
      show(null);
      setPhase("streaming");
      const outcome = await follow(open, null);
      switch (outcome.kind) {
        case "done":
        case "idle":
          await settle();
          return true;
        case "dropped":
          return lost(outcome.events);
        case "failed":
          return refused(outcome.error, outcome.events);
        case "aborted":
        case "restart":
          return true;
      }
    },
    [follow, lost, refused, settle, show]
  );

  const send = useCallback(
    (text: string, files: Attached[] = [], editFrom?: string) => {
      const list = savedRef.current ?? [];
      const cut = editFrom ? list.findIndex((message) => message.id === editFrom) : -1;
      const kept = cut === -1 ? list : list.slice(0, cut);
      return start(
        (onEvent, signal) =>
          sendMessage(
            chatId,
            { text, fileIds: files.map((file) => file.fileId), ...(editFrom ? { editFrom } : {}) },
            onEvent,
            signal
          ).then(() => true),
        {
          cut: editFrom ?? null,
          message: draftMessage(text, files),
          after: kept.at(-1)?.id ?? null,
        }
      );
    },
    [chatId, start]
  );

  const again = useCallback(
    () =>
      start((onEvent, signal) => regenerate(chatId, onEvent, signal).then(() => true), {
        cut: regenerateCut(savedRef.current ?? []),
        message: null,
        after: null,
      }),
    [chatId, start]
  );

  const retryTurn = useCallback(
    () =>
      start((onEvent, signal) => retry(chatId, onEvent, signal).then(() => true), {
        cut: retryCut(savedRef.current ?? []),
        message: null,
        after: null,
      }),
    [chatId, start]
  );

  const choose = useCallback(
    async (messageId: string, decisions: CallDecision[]) => {
      const key = keys.chatMessages(chatId);
      const before = client.getQueryData<MessageResponse[]>(key);
      // The cards close at once, as the server records the decisions.
      if (before) client.setQueryData(key, decided(before, messageId, decisions));
      const ok = await start(
        (onEvent, signal) =>
          decide(chatId, messageId, { decisions }, onEvent, signal).then(() => true),
        null
      );
      if (!ok) await client.invalidateQueries({ queryKey: key });
      return ok;
    },
    [chatId, client, start]
  );

  const halt = useCallback(async () => {
    try {
      await stop(chatId);
      // The stopped answer is saved now; without a stream to end the run, read it.
      if (!connection.current && current.current) await settle();
    } catch (error) {
      setAlert(describeError(error).message);
    }
  }, [chatId, settle]);

  // Leaving the page stops reading. The abort waits a tick, so a remount right away (React's
  // strict mode) keeps the stream.
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      const open = connection.current;
      setTimeout(() => {
        if (!mounted.current) open?.abort();
      }, 0);
    };
  }, []);

  // Opening the chat attaches to a turn that is running (from another device, or before a reload).
  const opened = useRef(false);
  useEffect(() => {
    if (opened.current || !attachOnOpen) return;
    opened.current = true;
    if (!connection.current) void reattach(null);
  }, [attachOnOpen, reattach]);

  // After a dropped connection, attach again after a pause, or at once when the device comes back
  // online or the page becomes visible; an idle page looks for a run started meanwhile then too.
  useEffect(() => {
    const now = () => {
      if (connection.current) return;
      void reattach(phase === "reconnecting" ? current.current?.seq || null : null);
    };
    const visible = () => {
      if (document.visibilityState === "visible") now();
    };
    const timer = phase === "reconnecting" ? setTimeout(now, backoff(attempt - 1)) : undefined;
    window.addEventListener("online", now);
    document.addEventListener("visibilitychange", visible);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("online", now);
      document.removeEventListener("visibilitychange", visible);
    };
  }, [attempt, phase, reattach]);

  return {
    run,
    draft,
    phase,
    alert,
    windowStartId,
    send,
    regenerate: again,
    retry: retryTurn,
    decide: choose,
    stop: halt,
  };
}
