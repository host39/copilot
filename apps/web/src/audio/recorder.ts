// Push to talk's recorder (spec, Voice): the microphone through an AudioWorklet
// (`public/audio/recorder.js`), resampled to 16 kHz mono and kept as PCM16 until the clip stops
// or reaches its 5-minute cap. No codec library and no MediaRecorder (Chromium's records
// WebM/Opus, which `input_audio` does not take).

import { Resampler } from "@web/audio/resample";
import { Clip, pcm16, SAMPLE_RATE } from "@web/audio/wav";

/** The worklet, served from the app's own origin. */
export const WORKLET_URL = "/audio/recorder.js";

/** The worklet's processor name. */
const PROCESSOR = "copilot-recorder";

/** How long `stop` waits for the worklet's last samples. */
const STOP_WAIT_MS = 1_000;

/** What the browser offers for recording. */
export type MicSupport = { secure: boolean; mediaDevices: boolean; worklet: boolean };

/** This browser's support, read from its globals. */
export function micSupport(): MicSupport {
  return {
    secure: typeof isSecureContext === "undefined" || isSecureContext,
    mediaDevices:
      typeof navigator !== "undefined" &&
      typeof navigator.mediaDevices?.getUserMedia === "function",
    worklet: typeof AudioWorkletNode !== "undefined" && typeof AudioContext !== "undefined",
  };
}

/** Why this browser cannot record, or `null` when it can. */
export function unsupported(support: MicSupport): string | null {
  // Browsers hide the microphone from pages served over plain HTTP (except localhost).
  if (!support.secure) {
    return "Recording needs a secure connection. Open Copilot over HTTPS to use the microphone.";
  }
  if (!support.mediaDevices) return "This browser cannot use a microphone.";
  if (!support.worklet) return "This browser cannot record audio.";
  return null;
}

/** What to tell the user when the microphone could not start (`getUserMedia` and its errors). */
export function micProblem(error: unknown): string {
  const name =
    typeof error === "object" && error !== null && "name" in error ? String(error.name) : "";
  switch (name) {
    case "NotAllowedError":
    case "SecurityError":
      return "Microphone access is blocked. Allow the microphone for this site in the browser's settings, then try again.";
    case "NotFoundError":
    case "OverconstrainedError":
      return "No microphone was found. Connect one and try again.";
    case "NotReadableError":
    case "AbortError":
      return "The microphone could not start. Another app may be using it.";
    default:
      return "Recording could not start.";
  }
}

/** How a recording ended by itself. */
export type Ending = "full" | "lost";

export type RecordListener = {
  /** New samples arrived; the clip holds `ms` of audio. */
  progress: (ms: number) => void;
  /** The clip reached its cap (`full`) or the microphone went away (`lost`); call `stop`. */
  ended: (why: Ending) => void;
};

/** A recording in progress. */
export type Recording = {
  /** Ends the recording and returns the clip (as much as was recorded). */
  stop: () => Promise<Clip>;
  /** Ends the recording and drops it. */
  cancel: () => void;
};

type WorkletMessage = { type: "samples"; samples: Float32Array } | { type: "end" };

/**
 * Starts recording. Throws when the microphone cannot start: `micProblem` says why. The
 * microphone is released once the recording stops or is cancelled.
 */
export async function record(listener: RecordListener): Promise<Recording> {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: {
      channelCount: 1,
      echoCancellation: true,
      noiseSuppression: true,
      autoGainControl: true,
    },
  });
  let context: AudioContext | null = null;
  const release = () => {
    for (const track of stream.getTracks()) track.stop();
    void context?.close().catch(() => undefined);
  };

  try {
    context = new AudioContext({ latencyHint: "interactive" });
    await context.audioWorklet.addModule(WORKLET_URL);
    // Recording follows a click, so the context may run; a refused resume leaves the timer at 0.
    if (context.state === "suspended") void context.resume().catch(() => undefined);
    const source = context.createMediaStreamSource(stream);
    const node = new AudioWorkletNode(context, PROCESSOR, {
      numberOfInputs: 1,
      numberOfOutputs: 1,
      outputChannelCount: [1],
    });
    const clip = new Clip();
    const resampler = new Resampler(context.sampleRate, SAMPLE_RATE);
    let finished: (() => void) | null = null;
    let over = false;

    const end = (why: Ending) => {
      if (over) return;
      over = true;
      listener.ended(why);
    };

    node.port.onmessage = (event: MessageEvent<WorkletMessage>) => {
      if (event.data.type === "end") {
        finished?.();
        return;
      }
      if (clip.full) return;
      if (clip.add(pcm16(resampler.push(event.data.samples)))) end("full");
      listener.progress(clip.durationMs);
    };
    for (const track of stream.getAudioTracks()) track.onended = () => end("lost");

    source.connect(node);
    // The node writes silence; being connected keeps the graph pulling it in every browser.
    node.connect(context.destination);

    const close = () => {
      node.port.onmessage = null;
      source.disconnect();
      node.disconnect();
      release();
    };

    return {
      stop: async () => {
        over = true;
        await new Promise<void>((resolve) => {
          const timer = setTimeout(resolve, STOP_WAIT_MS);
          finished = () => {
            clearTimeout(timer);
            resolve();
          };
          node.port.postMessage("stop");
        });
        close();
        return clip;
      },
      cancel: () => {
        over = true;
        close();
      },
    };
  } catch (error) {
    release();
    throw error;
  }
}
