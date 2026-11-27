// Push to talk's clip format (spec, Voice): 16 kHz mono PCM16 WAV, encoded here without a codec
// library. Samples arrive as floats from the recorder, become 16-bit integers, and gather in a
// clip that stops at the 5-minute cap; the WAV file is a 44-byte header before the samples.

/** Core's `AUDIO_SAMPLE_RATE`: the rate clips are recorded at. */
export const SAMPLE_RATE = 16_000;

/** Core's `AUDIO_CLIP_MAX_SECS`: the longest clip. */
export const CLIP_MAX_SECS = 300;

/** The most samples a clip holds. */
export const CLIP_MAX_SAMPLES = SAMPLE_RATE * CLIP_MAX_SECS;

/** The size of a canonical PCM WAV header (RIFF, `fmt `, and `data` chunk headers). */
export const WAV_HEADER_BYTES = 44;

/** One float sample (`-1` to `1`) as a 16-bit integer; louder samples clip, and `NaN` is silence. */
export function toInt16(sample: number): number {
  if (Number.isNaN(sample)) return 0;
  const clamped = Math.max(-1, Math.min(1, sample));
  return clamped < 0 ? Math.round(clamped * 0x8000) : Math.round(clamped * 0x7fff);
}

/** Float samples as 16-bit integers. */
export function pcm16(samples: Float32Array): Int16Array {
  const out = new Int16Array(samples.length);
  for (let index = 0; index < samples.length; index += 1) {
    out[index] = toInt16(samples[index] ?? 0);
  }
  return out;
}

function ascii(view: DataView, offset: number, text: string) {
  for (let index = 0; index < text.length; index += 1) {
    view.setUint8(offset + index, text.charCodeAt(index));
  }
}

/** The header of a mono PCM16 WAV file whose samples take `dataBytes`. */
export function wavHeader(dataBytes: number, sampleRate = SAMPLE_RATE): ArrayBuffer {
  const channels = 1;
  const bytesPerSample = 2;
  const buffer = new ArrayBuffer(WAV_HEADER_BYTES);
  const view = new DataView(buffer);
  ascii(view, 0, "RIFF");
  view.setUint32(4, 36 + dataBytes, true);
  ascii(view, 8, "WAVE");
  ascii(view, 12, "fmt ");
  view.setUint32(16, 16, true); // the size of the fmt chunk
  view.setUint16(20, 1, true); // PCM
  view.setUint16(22, channels, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * channels * bytesPerSample, true); // bytes per second
  view.setUint16(32, channels * bytesPerSample, true); // bytes per frame
  view.setUint16(34, bytesPerSample * 8, true); // bits per sample
  ascii(view, 36, "data");
  view.setUint32(40, dataBytes, true);
  return buffer;
}

/**
 * Recorded samples, gathered up to `max` (the 5-minute cap). Samples past the cap are dropped and
 * the clip reports itself `full`, so the recorder stops.
 */
export class Clip {
  private readonly chunks: Int16Array[] = [];
  private count = 0;

  constructor(
    readonly sampleRate = SAMPLE_RATE,
    readonly max = CLIP_MAX_SAMPLES
  ) {}

  /** The samples so far. */
  get length(): number {
    return this.count;
  }

  /** The clip reached its cap. */
  get full(): boolean {
    return this.count >= this.max;
  }

  /** How long the clip runs, in milliseconds. */
  get durationMs(): number {
    return Math.round((this.count / this.sampleRate) * 1000);
  }

  /** Adds samples up to the cap; `true` when the clip is full afterwards. */
  add(samples: Int16Array): boolean {
    const room = this.max - this.count;
    if (room > 0 && samples.length > 0) {
      const kept = samples.length > room ? samples.slice(0, room) : samples;
      this.chunks.push(kept);
      this.count += kept.length;
    }
    return this.full;
  }

  /** The clip as a WAV file. */
  wav(): Blob {
    const parts: BlobPart[] = [wavHeader(this.count * 2, this.sampleRate)];
    for (const chunk of this.chunks) parts.push(littleEndian(chunk));
    return new Blob(parts, { type: "audio/wav" });
  }
}

/** The bytes of `samples` in little-endian order (WAV's), whatever the platform's order is. */
function littleEndian(samples: Int16Array): ArrayBuffer {
  const buffer = new ArrayBuffer(samples.length * 2);
  const view = new DataView(buffer);
  for (let index = 0; index < samples.length; index += 1) {
    view.setInt16(index * 2, samples[index] ?? 0, true);
  }
  return buffer;
}

/** A clip length as `m:ss` (`0:07`, `5:00`). */
export function clock(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

/** The file name of a clip recorded at `at`, in the device's local time. */
export function clipName(at: Date): string {
  const pad = (value: number) => String(value).padStart(2, "0");
  const date = `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`;
  return `Voice message ${date} ${pad(at.getHours())}-${pad(at.getMinutes())}-${pad(at.getSeconds())}.wav`;
}
