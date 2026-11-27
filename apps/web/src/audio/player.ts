// Spoken replies as they stream (spec, Voice): `audio.delta` events carry base64 PCM16, which
// plays through Web Audio with little delay. Each chunk becomes a buffer scheduled right where the
// previous one ends, so the voice runs without gaps; the first chunk, and one that arrives after
// the queue ran dry, starts a short lead from now so the next chunk has time to arrive.

/** The rate of `pcm16` audio: OpenAI's PCM16 is 24 kHz mono, little-endian. */
export const PCM16_RATE = 24_000;

/** How far ahead the first chunk after a pause starts, in seconds (a small jitter buffer). */
export const LEAD_SECS = 0.1;

/** Base64 text as bytes. */
export function base64Bytes(data: string): Uint8Array {
  const text = atob(data);
  const bytes = new Uint8Array(text.length);
  for (let index = 0; index < text.length; index += 1) bytes[index] = text.charCodeAt(index);
  return bytes;
}

/**
 * PCM16 little-endian bytes as float samples (`-1` to `1`). Chunks may split a sample: an odd
 * byte at the end waits for the next chunk.
 */
export class Pcm16Decoder {
  private pending: number | null = null;

  decode(bytes: Uint8Array): Float32Array {
    let joined = bytes;
    if (this.pending !== null) {
      joined = new Uint8Array(bytes.length + 1);
      joined[0] = this.pending;
      joined.set(bytes, 1);
    }
    const whole = joined.length - (joined.length % 2);
    this.pending = whole < joined.length ? (joined[whole] ?? 0) : null;
    const view = new DataView(joined.buffer, joined.byteOffset, whole);
    const out = new Float32Array(whole / 2);
    for (let index = 0; index < out.length; index += 1) {
      out[index] = view.getInt16(index * 2, true) / 0x8000;
    }
    return out;
  }

  reset() {
    this.pending = null;
  }
}

/**
 * When a chunk of `duration` seconds plays: where the queue ends, or `lead` after `now` when the
 * queue has run dry (or never started).
 */
export function slot(
  queueEnd: number,
  now: number,
  duration: number,
  lead = LEAD_SECS
): { start: number; end: number } {
  const start = queueEnd > now ? queueEnd : now + lead;
  return { start, end: start + duration };
}

/** What the player needs of an `AudioContext`. */
export type AudioOutput = {
  readonly currentTime: number;
  readonly state: string;
  readonly destination: AudioNode;
  createBuffer(channels: number, length: number, sampleRate: number): AudioBuffer;
  createBufferSource(): AudioBufferSourceNode;
  resume(): Promise<void>;
  close(): Promise<void>;
};

/** A new `AudioContext` for playback, or `null` where the browser has none. */
export function audioOutput(): AudioOutput | null {
  if (typeof AudioContext === "undefined") return null;
  return new AudioContext({ latencyHint: "interactive" });
}

/** Stops whatever played last, so only one recording or reply plays at a time. */
let playing: (() => void) | null = null;

/**
 * Starts playing something that stops with `stop`: whatever played before stops. Returns the
 * release to call when it ends on its own.
 */
export function claim(stop: () => void): () => void {
  if (playing !== stop) playing?.();
  playing = stop;
  return () => {
    if (playing === stop) playing = null;
  };
}

/**
 * A queue of PCM16 chunks played back to back. The audio context is made on first use (or by
 * `unlock`, which a click calls, since browsers start audio only after a user gesture).
 */
export class Player {
  private output: AudioOutput | null = null;
  private readonly decoder = new Pcm16Decoder();
  private readonly sources = new Set<AudioBufferSourceNode>();
  private queueEnd = 0;
  private release: (() => void) | null = null;
  private readonly halt = () => this.stop();

  constructor(private readonly create: () => AudioOutput | null = audioOutput) {}

  private context(): AudioOutput | null {
    this.output ??= this.create();
    return this.output;
  }

  /** Makes the audio context and lets it run; call it from a click. */
  unlock() {
    const output = this.context();
    if (output?.state === "suspended") void output.resume().catch(() => undefined);
  }

  /** Whether chunks are still queued or playing. */
  get busy(): boolean {
    return this.sources.size > 0;
  }

  /** Queues one `audio.delta` (`format` `pcm16`; other formats are skipped). */
  play(data: string, format: string) {
    if (format !== "pcm16") return;
    const output = this.context();
    if (!output) return;
    const samples = this.decoder.decode(base64Bytes(data));
    if (samples.length === 0) return;
    if (output.state === "suspended") void output.resume().catch(() => undefined);

    const buffer = output.createBuffer(1, samples.length, PCM16_RATE);
    buffer.getChannelData(0).set(samples);
    const source = output.createBufferSource();
    source.buffer = buffer;
    source.connect(output.destination);
    const { start, end } = slot(this.queueEnd, output.currentTime, buffer.duration);
    source.onended = () => {
      this.sources.delete(source);
      if (this.sources.size === 0) this.ended();
    };
    this.sources.add(source);
    this.queueEnd = end;
    this.release ??= claim(this.halt);
    source.start(start);
  }

  private ended() {
    this.release?.();
    this.release = null;
  }

  /** Stops what plays and everything queued. */
  stop() {
    for (const source of this.sources) {
      source.onended = null;
      try {
        source.stop();
      } catch {
        // It never started, or ended already.
      }
      source.disconnect();
    }
    this.sources.clear();
    this.queueEnd = 0;
    this.decoder.reset();
    this.ended();
  }

  /** Stops and lets the audio context go. */
  close() {
    this.stop();
    const output = this.output;
    this.output = null;
    void output?.close().catch(() => undefined);
  }
}
