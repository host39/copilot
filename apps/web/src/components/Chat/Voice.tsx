import { useMutation } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { fetchFileContent } from "@web/api/routes/files";
import { claim } from "@web/audio/player";
import { CLIP_MAX_SECS, clock } from "@web/audio/wav";
import type { MicState } from "@web/hooks/useRecorder";
import { type Ref, useEffect, useRef, useState } from "react";
import { LuMic, LuPlay, LuSquare, LuVolume2, LuVolumeX, LuX } from "react-icons/lu";

/** The microphone button, shown only for models with `audioIn`. */
export function MicButton({
  onClick,
  disabled,
  ref,
}: {
  onClick: () => void;
  disabled?: boolean;
  ref?: Ref<HTMLButtonElement>;
}) {
  return (
    <button
      ref={ref}
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-label="Record a voice message"
      title="Record a voice message"
      className="icon-button"
    >
      <LuMic aria-hidden />
    </button>
  );
}

/** The time a recording shows: what was recorded, and the cap (`0:12 / 5:00`). */
export function recordingTime(ms: number): string {
  return `${clock(ms)} / ${clock(CLIP_MAX_SECS * 1000)}`;
}

/**
 * The bar that replaces the message field while recording: the timer against the 5-minute cap,
 * Cancel (or Escape), and Stop, which keeps the clip for a preview before it is sent.
 */
export function RecordingBar({
  state,
  onStop,
  onCancel,
}: {
  state: Exclude<MicState, { kind: "idle" }>;
  onStop: () => void;
  onCancel: () => void;
}) {
  const stopButton = useRef<HTMLButtonElement>(null);
  const cancelButton = useRef<HTMLButtonElement>(null);
  const recording = state.kind === "recording";
  // The microphone button is gone: keep focus in the bar, so Escape and Enter reach it.
  useEffect(() => {
    if (recording) stopButton.current?.focus();
    else cancelButton.current?.focus();
  }, [recording]);
  const label =
    state.kind === "starting"
      ? "Waiting for the microphone..."
      : state.kind === "stopping"
        ? "Finishing..."
        : "Recording";
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: Escape anywhere in the bar cancels
    <div
      className="flex min-h-11 min-w-0 flex-1 items-center gap-2 rounded-lg border border-line bg-canvas px-2"
      onKeyDown={(event) => {
        if (event.key !== "Escape") return;
        // The chat page's Escape stops an answer; here it only cancels the recording.
        event.preventDefault();
        onCancel();
      }}
    >
      <button
        ref={cancelButton}
        type="button"
        onClick={onCancel}
        aria-label="Cancel recording"
        title="Cancel"
        className="flex size-8 shrink-0 items-center justify-center rounded-md text-muted hover:bg-raised hover:text-fg"
      >
        <LuX aria-hidden />
      </button>
      <span
        aria-hidden
        className={`size-2.5 shrink-0 rounded-full ${recording ? "animate-pulse bg-danger" : "bg-line"}`}
      />
      <span role="status" className="min-w-0 flex-1 truncate text-sm">
        {label}
      </span>
      {state.kind === "starting" ? null : (
        <span role="timer" className="shrink-0 text-sm text-muted tabular-nums">
          {recordingTime(state.ms)}
        </span>
      )}
      <button
        ref={stopButton}
        type="button"
        onClick={onStop}
        disabled={!recording}
        aria-label="Stop recording"
        title="Stop"
        className="flex size-8 shrink-0 items-center justify-center rounded-md text-danger hover:bg-raised disabled:opacity-60"
      >
        <LuSquare aria-hidden />
      </button>
    </div>
  );
}

/** "Speak replies", shown only for models with `audioOut`: the chat's answers come with audio. */
export function SpeakToggle({
  on,
  onChange,
  disabled,
}: {
  on: boolean;
  onChange: (on: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={() => onChange(!on)}
      disabled={disabled}
      aria-pressed={on}
      aria-label="Speak replies"
      title={on ? "Speak replies: on" : "Speak replies: off"}
      className={`icon-button ${on ? "text-accent" : ""}`}
    >
      {on ? <LuVolume2 aria-hidden /> : <LuVolumeX aria-hidden />}
    </button>
  );
}

/** Keeps a playing `<audio>` the only sound: another recording or a spoken reply stops it. */
function useExclusive() {
  const release = useRef<(() => void) | null>(null);
  useEffect(() => () => release.current?.(), []);
  return {
    onPlay: (event: { currentTarget: HTMLAudioElement }) => {
      const element = event.currentTarget;
      release.current?.();
      release.current = claim(() => element.pause());
    },
    onPause: () => {
      release.current?.();
      release.current = null;
    },
  };
}

/** A local recording or audio file before it is sent: the browser's player over its bytes. */
export function AudioPreview({ file, label }: { file: Blob; label: string }) {
  const [url, setUrl] = useState<string | null>(null);
  const exclusive = useExclusive();
  useEffect(() => {
    const next = URL.createObjectURL(file);
    setUrl(next);
    return () => URL.revokeObjectURL(next);
  }, [file]);
  if (!url) return null;
  return (
    <audio controls src={url} aria-label={label} className="h-9 w-64 max-w-full" {...exclusive} />
  );
}

/**
 * A saved recording or spoken reply (an `audio` part): Play fetches its bytes with the token
 * (never in a URL) and plays them in the browser's player, which can replay and seek.
 */
export function AudioPart({
  fileId,
  label,
  durationMs,
}: {
  fileId: string;
  label: string;
  durationMs: number | null;
}) {
  const [url, setUrl] = useState<string | null>(null);
  const exclusive = useExclusive();
  const load = useMutation({
    mutationFn: () => fetchFileContent(fileId),
    onSuccess: (blob) => setUrl(URL.createObjectURL(blob)),
  });
  useEffect(() => () => (url ? URL.revokeObjectURL(url) : undefined), [url]);

  if (url) {
    return (
      <audio
        controls
        autoPlay
        src={url}
        aria-label={label}
        className="h-9 w-64 max-w-full"
        {...exclusive}
      />
    );
  }
  return (
    <span className="inline-flex flex-col">
      <button
        type="button"
        onClick={() => load.mutate()}
        disabled={load.isPending}
        aria-label={`Play ${label.toLowerCase()}`}
        title={`Play ${label.toLowerCase()}`}
        className="inline-flex min-h-9 items-center gap-2 rounded-lg border border-line bg-canvas px-3 text-sm hover:bg-raised disabled:opacity-60"
      >
        <LuPlay aria-hidden className="shrink-0 text-muted" />
        <span>{label}</span>
        {durationMs !== null ? (
          <span className="text-muted tabular-nums">{clock(durationMs)}</span>
        ) : null}
      </button>
      {load.isError ? (
        <span role="alert" className="field-error">
          {describeError(load.error).message}
        </span>
      ) : null}
    </span>
  );
}
