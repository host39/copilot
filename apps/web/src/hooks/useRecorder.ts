// Push to talk in the composer: the microphone's state, its timer, Stop and Cancel, and what the
// user is told when recording cannot start or ended by itself. A finished clip becomes a WAV
// file that the composer attaches (and previews) like any other file.

import {
  type Ending,
  type MicHost,
  micProblem,
  micSupport,
  type Recording,
  record,
  unsupported,
} from "@web/audio/recorder";
import { clipName } from "@web/audio/wav";
import { isAndroid, isNative } from "@web/config";
import { log } from "@web/logging";
import { useCallback, useEffect, useReducer, useRef, useState } from "react";

/** Where the microphone stands. */
export type MicState =
  | { kind: "idle" }
  /** Waiting for the microphone (the browser may be asking for permission). */
  | { kind: "starting" }
  | { kind: "recording"; ms: number }
  /** Stop was pressed; the last samples are on their way. */
  | { kind: "stopping"; ms: number };

export type MicAction =
  | { type: "start" }
  | { type: "started" }
  | { type: "progress"; ms: number }
  | { type: "stop" }
  /** Stopped, cancelled, or failed. */
  | { type: "reset" };

/**
 * The microphone after `action`. Progress changes the state only when the timer's second
 * changes, so the composer redraws once a second while recording.
 */
export function mic(state: MicState, action: MicAction): MicState {
  switch (action.type) {
    case "start":
      return state.kind === "idle" ? { kind: "starting" } : state;
    case "started":
      return state.kind === "starting" ? { kind: "recording", ms: 0 } : state;
    case "progress": {
      if (state.kind !== "recording") return state;
      const second = Math.floor(action.ms / 1000);
      return second === Math.floor(state.ms / 1000) ? state : { kind: "recording", ms: action.ms };
    }
    case "stop":
      return state.kind === "recording" ? { kind: "stopping", ms: state.ms } : state;
    case "reset":
      return { kind: "idle" };
  }
}

/** Clips shorter than this are a slip of the finger, not a message. */
export const MIN_CLIP_MS = 300;

/** A line under the composer: a problem (the microphone did not start) or a note. */
export type MicNote = { text: string; problem: boolean };

/** Whether a stopped clip is kept, and what the user is told about how it ended. */
export function clipOutcome(
  durationMs: number,
  ending: Ending | null
): { keep: boolean; note: MicNote | null } {
  if (durationMs < MIN_CLIP_MS) {
    return { keep: false, note: { text: "The recording was too short.", problem: false } };
  }
  if (ending === "full") {
    return {
      keep: true,
      note: { text: "Recording stopped at the 5-minute limit.", problem: false },
    };
  }
  if (ending === "lost") {
    return {
      keep: true,
      note: { text: "The microphone stopped, so the recording ends there.", problem: false },
    };
  }
  return { keep: true, note: null };
}

/** Where this page runs, for the advice on a blocked microphone. */
export function micHost(): MicHost {
  if (isAndroid()) return "android";
  return isNative() ? "desktop" : "browser";
}

/**
 * The recorder: `start` asks for the microphone, `stop` hands the clip to `onClip` as a WAV file,
 * `cancel` drops it. Leaving the page drops a recording in progress and releases the microphone.
 */
export function useRecorder(onClip: (file: File) => void) {
  const [state, dispatch] = useReducer(mic, { kind: "idle" });
  const [note, setNote] = useState<MicNote | null>(null);
  const recording = useRef<Recording | null>(null);
  const ending = useRef<Ending | null>(null);
  // Each start and cancel moves it, so a start or stop that resolves after Cancel drops its clip.
  const attempt = useRef(0);
  const alive = useRef(true);
  const deliver = useRef(onClip);
  deliver.current = onClip;

  const stop = useCallback(async () => {
    const current = recording.current;
    if (!current) return;
    recording.current = null;
    const mine = attempt.current;
    dispatch({ type: "stop" });
    const clip = await current.stop();
    // Cancel while the last samples arrived, or leaving the page, drops the clip.
    if (!alive.current || attempt.current !== mine) return;
    dispatch({ type: "reset" });
    const outcome = clipOutcome(clip.durationMs, ending.current);
    setNote(outcome.note);
    if (outcome.keep) {
      deliver.current(new File([clip.wav()], clipName(new Date()), { type: "audio/wav" }));
    }
  }, []);

  const start = useCallback(async () => {
    if (state.kind !== "idle") return;
    setNote(null);
    const reason = unsupported(micSupport());
    if (reason) {
      setNote({ text: reason, problem: true });
      return;
    }
    attempt.current += 1;
    const mine = attempt.current;
    ending.current = null;
    dispatch({ type: "start" });
    try {
      const started = await record({
        progress: (ms) => dispatch({ type: "progress", ms }),
        ended: (why) => {
          ending.current = why;
          void stop();
        },
      });
      if (!alive.current || attempt.current !== mine) {
        started.cancel();
        return;
      }
      recording.current = started;
      dispatch({ type: "started" });
    } catch (error) {
      if (attempt.current !== mine) return;
      dispatch({ type: "reset" });
      setNote({ text: micProblem(error, micHost()), problem: true });
      log("warn", "web.voice.record.failed", {
        error: error instanceof Error ? error.name : String(error),
      });
    }
  }, [state.kind, stop]);

  const cancel = useCallback(() => {
    attempt.current += 1;
    recording.current?.cancel();
    recording.current = null;
    dispatch({ type: "reset" });
    setNote(null);
  }, []);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      attempt.current += 1;
      recording.current?.cancel();
      recording.current = null;
    };
  }, []);

  return { state, note, start, stop, cancel, dismiss: () => setNote(null) };
}
