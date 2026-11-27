// Voice: the WAV encoder (header, sample conversion, the 5-minute cap), the resampler, the
// recorder's messages and its worklet, and the player queue for spoken replies.

import { readFileSync } from "node:fs";
import { describe, expect, test } from "@rstest/core";
import {
  type AudioOutput,
  base64Bytes,
  claim,
  LEAD_SECS,
  PCM16_RATE,
  Pcm16Decoder,
  Player,
  slot,
} from "@web/audio/player";
import { micProblem, unsupported, WORKLET_URL } from "@web/audio/recorder";
import { Resampler } from "@web/audio/resample";
import {
  CLIP_MAX_SAMPLES,
  CLIP_MAX_SECS,
  Clip,
  clipName,
  clock,
  pcm16,
  SAMPLE_RATE,
  toInt16,
  WAV_HEADER_BYTES,
  wavHeader,
} from "@web/audio/wav";

/** The four ASCII letters at `offset`. */
function tag(view: DataView, offset: number): string {
  return String.fromCharCode(...[0, 1, 2, 3].map((index) => view.getUint8(offset + index)));
}

describe("WAV header", () => {
  test("is a 44-byte RIFF WAVE header for 16 kHz mono PCM16", () => {
    const view = new DataView(wavHeader(32_000));
    expect(view.byteLength).toBe(WAV_HEADER_BYTES);
    expect(tag(view, 0)).toBe("RIFF");
    expect(view.getUint32(4, true)).toBe(36 + 32_000);
    expect(tag(view, 8)).toBe("WAVE");
    expect(tag(view, 12)).toBe("fmt ");
    expect(view.getUint32(16, true)).toBe(16);
    expect(view.getUint16(20, true)).toBe(1); // PCM
    expect(view.getUint16(22, true)).toBe(1); // mono
    expect(view.getUint32(24, true)).toBe(16_000);
    expect(view.getUint32(28, true)).toBe(32_000); // bytes per second
    expect(view.getUint16(32, true)).toBe(2); // bytes per frame
    expect(view.getUint16(34, true)).toBe(16); // bits per sample
    expect(tag(view, 36)).toBe("data");
    expect(view.getUint32(40, true)).toBe(32_000);
  });

  test("names the rate it is given", () => {
    const view = new DataView(wavHeader(0, 24_000));
    expect(view.getUint32(24, true)).toBe(24_000);
    expect(view.getUint32(28, true)).toBe(48_000);
    expect(view.getUint32(4, true)).toBe(36);
  });

  test("clips are 16 kHz and at most five minutes, as core's constants say", () => {
    expect(SAMPLE_RATE).toBe(16_000);
    expect(CLIP_MAX_SECS).toBe(300);
    expect(CLIP_MAX_SAMPLES).toBe(4_800_000);
  });
});

describe("sample conversion", () => {
  test("floats become 16-bit integers over the whole range", () => {
    expect(toInt16(0)).toBe(0);
    expect(toInt16(1)).toBe(32_767);
    expect(toInt16(-1)).toBe(-32_768);
    expect(toInt16(0.5)).toBe(16_384);
    expect(toInt16(-0.5)).toBe(-16_384);
  });

  test("louder samples clip and NaN is silence", () => {
    expect(toInt16(1.7)).toBe(32_767);
    expect(toInt16(-3)).toBe(-32_768);
    expect(toInt16(Number.NaN)).toBe(0);
    expect(toInt16(Number.POSITIVE_INFINITY)).toBe(32_767);
  });

  test("pcm16 converts every sample in order", () => {
    expect(Array.from(pcm16(Float32Array.from([0, 1, -1, 2, 0.25])))).toEqual([
      0, 32_767, -32_768, 32_767, 8_192,
    ]);
  });
});

describe("clip", () => {
  test("encodes its samples as a little-endian WAV file", async () => {
    const clip = new Clip();
    clip.add(Int16Array.from([1, -2]));
    clip.add(Int16Array.from([256]));
    const blob = clip.wav();
    expect(blob.type).toBe("audio/wav");
    expect(blob.size).toBe(WAV_HEADER_BYTES + 6);
    const view = new DataView(await blob.arrayBuffer());
    expect(view.getUint32(40, true)).toBe(6);
    expect(view.getUint32(4, true)).toBe(42);
    expect([view.getInt16(44, true), view.getInt16(46, true), view.getInt16(48, true)]).toEqual([
      1, -2, 256,
    ]);
    // Little-endian: the low byte first.
    expect([view.getUint8(48), view.getUint8(49)]).toEqual([0, 1]);
  });

  test("measures its length in samples and milliseconds", () => {
    const clip = new Clip();
    expect([clip.length, clip.durationMs, clip.full]).toEqual([0, 0, false]);
    clip.add(new Int16Array(8_000));
    expect([clip.length, clip.durationMs]).toEqual([8_000, 500]);
  });

  test("stops at the cap: later samples are dropped and the clip reports itself full", async () => {
    const clip = new Clip(SAMPLE_RATE, 10);
    expect(clip.add(new Int16Array(6))).toBe(false);
    expect(clip.add(Int16Array.from([1, 2, 3, 4, 5, 6]))).toBe(true);
    expect(clip.length).toBe(10);
    expect(clip.add(new Int16Array(4))).toBe(true);
    expect(clip.length).toBe(10);
    const view = new DataView(await clip.wav().arrayBuffer());
    expect(view.byteLength).toBe(WAV_HEADER_BYTES + 20);
    expect(view.getUint32(40, true)).toBe(20);
    expect(view.getInt16(WAV_HEADER_BYTES + 18, true)).toBe(4);
  });

  test("the default cap is five minutes of 16 kHz audio", () => {
    const clip = new Clip();
    const minute = new Int16Array(SAMPLE_RATE * 60);
    for (let index = 0; index < 4; index += 1) expect(clip.add(minute)).toBe(false);
    expect(clip.add(minute)).toBe(true);
    expect(clip.add(minute)).toBe(true);
    expect(clip.length).toBe(CLIP_MAX_SAMPLES);
    expect(clip.durationMs).toBe(300_000);
    expect(clip.wav().size).toBe(WAV_HEADER_BYTES + CLIP_MAX_SAMPLES * 2);
  });

  test("times read as m:ss", () => {
    expect(clock(0)).toBe("0:00");
    expect(clock(7_400)).toBe("0:07");
    expect(clock(61_000)).toBe("1:01");
    expect(clock(300_000)).toBe("5:00");
    expect(clock(-5)).toBe("0:00");
  });

  test("a clip's file name says when it was recorded", () => {
    expect(clipName(new Date(2026, 10, 27, 9, 5, 3))).toBe("Voice message 2026-11-27 09-05-03.wav");
  });
});

describe("resampler", () => {
  test("keeps samples at the same rate", () => {
    const resampler = new Resampler(16_000, 16_000);
    expect(Array.from(resampler.push(Float32Array.from([0.5, -0.25, 1])))).toEqual([0.5, -0.25, 1]);
  });

  test("48 kHz to 16 kHz averages each group of three samples", () => {
    const resampler = new Resampler(48_000, 16_000);
    const out = resampler.push(Float32Array.from([0.3, 0.6, 0.9, -0.3, -0.3, -0.3]));
    expect(Array.from(out).map((value) => Number(value.toFixed(5)))).toEqual([0.6, -0.3]);
  });

  test("input split across chunks gives the same output as one chunk", () => {
    const input = Float32Array.from({ length: 441 }, (_, index) => Math.sin(index / 7));
    const whole = new Resampler(44_100, 16_000).push(input);
    const split = new Resampler(44_100, 16_000);
    const parts = [split.push(input.slice(0, 100)), split.push(input.slice(100, 101))];
    parts.push(split.push(input.slice(101)));
    const joined = parts.flatMap((part) => Array.from(part));
    expect(joined.length).toBe(whole.length);
    joined.forEach((value, index) => {
      expect(value).toBeCloseTo(whole[index] ?? Number.NaN, 5);
    });
  });

  test("a second of 44.1 kHz audio becomes a second of 16 kHz audio", () => {
    const resampler = new Resampler(44_100, 16_000);
    let total = 0;
    for (let chunk = 0; chunk < 441; chunk += 1) {
      total += resampler.push(new Float32Array(100).fill(0.5)).length;
    }
    expect(Math.abs(total - 16_000)).toBeLessThanOrEqual(1);
  });

  test("a constant signal stays constant, so the level is kept", () => {
    const out = new Resampler(44_100, 16_000).push(new Float32Array(1_000).fill(-0.4));
    for (const value of out) expect(value).toBeCloseTo(-0.4, 5);
  });

  test("refuses a rate that is not positive", () => {
    expect(() => new Resampler(0, 16_000)).toThrow();
    expect(() => new Resampler(48_000, -1)).toThrow();
  });
});

describe("recorder messages", () => {
  test("a blocked or missing microphone says what to do", () => {
    const named = (name: string) => Object.assign(new Error("x"), { name });
    expect(micProblem(named("NotAllowedError"))).toContain("Microphone access is blocked");
    expect(micProblem(named("SecurityError"))).toContain("Microphone access is blocked");
    expect(micProblem(named("NotFoundError"))).toBe(
      "No microphone was found. Connect one and try again."
    );
    expect(micProblem(named("OverconstrainedError"))).toContain("No microphone was found");
    expect(micProblem(named("NotReadableError"))).toContain("Another app may be using it");
    expect(micProblem(named("TypeError"))).toBe("Recording could not start.");
    expect(micProblem("odd")).toBe("Recording could not start.");
  });

  test("a page without a secure connection, a microphone API, or AudioWorklet cannot record", () => {
    const all = { secure: true, mediaDevices: true, worklet: true };
    expect(unsupported(all)).toBeNull();
    expect(unsupported({ ...all, secure: false, mediaDevices: false })).toContain("HTTPS");
    expect(unsupported({ ...all, mediaDevices: false })).toBe(
      "This browser cannot use a microphone."
    );
    expect(unsupported({ ...all, worklet: false })).toBe("This browser cannot record audio.");
  });
});

/** The worklet file, run with stand-ins for the worklet scope. */
function loadWorklet() {
  const source = readFileSync(`public${WORKLET_URL}`, "utf8");
  const registered: Record<string, new () => WorkletProcessor> = {};
  class Base {
    port = {
      sent: [] as unknown[],
      onmessage: null as ((event: { data: unknown }) => void) | null,
      postMessage(message: unknown) {
        this.sent.push(message);
      },
    };
  }
  const register = (name: string, processor: new () => WorkletProcessor) => {
    registered[name] = processor;
  };
  new Function("AudioWorkletProcessor", "registerProcessor", source)(Base, register);
  return registered;
}

type WorkletProcessor = {
  port: {
    sent: unknown[];
    onmessage: ((event: { data: unknown }) => void) | null;
  };
  process(inputs: Float32Array[][]): boolean;
};

describe("recorder worklet", () => {
  test("mixes channels down to mono and posts the rest on stop, then ends", () => {
    const Processor = loadWorklet()["copilot-recorder"];
    expect(Processor).toBeDefined();
    if (!Processor) return;
    const node = new Processor();
    const left = Float32Array.from([1, 0.5]);
    const right = Float32Array.from([0, -0.5]);
    expect(node.process([[left, right]])).toBe(true);
    expect(node.port.sent).toEqual([]);
    node.port.onmessage?.({ data: "stop" });
    expect(node.port.sent).toEqual([
      { type: "samples", samples: Float32Array.from([0.5, 0]) },
      { type: "end" },
    ]);
    // Stopped: no more processing.
    expect(node.process([[left]])).toBe(false);
  });

  test("posts full chunks as they fill, and keeps running without input", () => {
    const Processor = loadWorklet()["copilot-recorder"];
    if (!Processor) throw new Error("not registered");
    const node = new Processor();
    for (let quantum = 0; quantum < 33; quantum += 1) {
      node.process([[new Float32Array(128).fill(0.25)]]);
    }
    expect(node.port.sent).toHaveLength(1);
    const first = node.port.sent[0] as { type: string; samples: Float32Array };
    expect(first.type).toBe("samples");
    expect(first.samples).toHaveLength(4096);
    expect(node.process([])).toBe(true);
  });
});

describe("PCM16 decoding", () => {
  test("base64 text becomes its bytes", () => {
    expect(Array.from(base64Bytes("AAH/fw=="))).toEqual([0, 1, 255, 127]);
  });

  test("little-endian samples become floats", () => {
    const decoder = new Pcm16Decoder();
    const out = decoder.decode(Uint8Array.from([0x00, 0x40, 0x00, 0x80, 0xff, 0x7f]));
    expect(Array.from(out)).toEqual([0.5, -1, 32_767 / 32_768]);
  });

  test("a sample split between chunks waits for its second byte", () => {
    const decoder = new Pcm16Decoder();
    expect(Array.from(decoder.decode(Uint8Array.from([0x00, 0x40, 0x00])))).toEqual([0.5]);
    expect(Array.from(decoder.decode(Uint8Array.from([0xc0])))).toEqual([-0.5]);
    decoder.decode(Uint8Array.from([0x01]));
    decoder.reset();
    expect(Array.from(decoder.decode(Uint8Array.from([0x00, 0x40])))).toEqual([0.5]);
  });
});

describe("player queue", () => {
  test("chunks play back to back, and a chunk after a pause starts a lead from now", () => {
    expect(slot(0, 0, 0.5)).toEqual({ start: LEAD_SECS, end: LEAD_SECS + 0.5 });
    expect(slot(1.2, 1, 0.5)).toEqual({ start: 1.2, end: 1.7 });
    expect(slot(1.2, 2, 0.5)).toEqual({ start: 2 + LEAD_SECS, end: 2.5 + LEAD_SECS });
    expect(slot(2, 2, 0.25, 0)).toEqual({ start: 2, end: 2.25 });
  });

  /** A stand-in `AudioContext` that records what the player schedules. */
  function fakeOutput() {
    const started: { start: number; samples: number[]; rate: number }[] = [];
    const sources: { stopped: boolean; ended: (() => void) | null }[] = [];
    const output = {
      currentTime: 0,
      state: "suspended",
      resumed: 0,
      closed: false,
      destination: {} as AudioNode,
      createBuffer(_channels: number, length: number, sampleRate: number) {
        const data = new Float32Array(length);
        return {
          duration: length / sampleRate,
          sampleRate,
          getChannelData: () => data,
        } as unknown as AudioBuffer;
      },
      createBufferSource() {
        const record = { stopped: false, ended: null as (() => void) | null };
        sources.push(record);
        const node = {
          buffer: null as AudioBuffer | null,
          connect: () => undefined,
          disconnect: () => undefined,
          set onended(handler: (() => void) | null) {
            record.ended = handler;
          },
          get onended() {
            return record.ended;
          },
          start(at: number) {
            const buffer = node.buffer as AudioBuffer;
            started.push({
              start: at,
              samples: Array.from(buffer.getChannelData(0)),
              rate: buffer.sampleRate,
            });
          },
          stop() {
            record.stopped = true;
          },
        };
        return node as unknown as AudioBufferSourceNode;
      },
      async resume() {
        output.resumed += 1;
        output.state = "running";
      },
      async close() {
        output.closed = true;
      },
    };
    return { output, started, sources };
  }

  /** Base64 of PCM16 samples. */
  function pcm(...samples: number[]): string {
    const bytes = new Uint8Array(samples.length * 2);
    const view = new DataView(bytes.buffer);
    samples.forEach((sample, index) => {
      view.setInt16(index * 2, sample, true);
    });
    return btoa(String.fromCharCode(...bytes));
  }

  test("queues each delta as a 24 kHz buffer right after the one before", () => {
    const { output, started } = fakeOutput();
    const player = new Player(() => output as AudioOutput);
    player.play(pcm(16_384, -16_384), "pcm16");
    player.play(pcm(0, 0, 0, 0), "pcm16");
    expect(started.map((item) => item.rate)).toEqual([PCM16_RATE, PCM16_RATE]);
    expect(started[0]?.samples).toEqual([0.5, -0.5]);
    expect(started[0]?.start).toBe(LEAD_SECS);
    expect(started[1]?.start).toBeCloseTo(LEAD_SECS + 2 / PCM16_RATE, 9);
    expect(player.busy).toBe(true);
    // A suspended context is asked to run.
    expect(output.resumed).toBeGreaterThan(0);
  });

  test("a delta after the queue ran dry starts a lead after now", () => {
    const { output, started } = fakeOutput();
    const player = new Player(() => output as AudioOutput);
    player.play(pcm(1, 2), "pcm16");
    output.currentTime = 5;
    player.play(pcm(3, 4), "pcm16");
    expect(started[1]?.start).toBe(5 + LEAD_SECS);
  });

  test("skips formats it cannot play and empty deltas", () => {
    const { output, started } = fakeOutput();
    const player = new Player(() => output as AudioOutput);
    player.play(pcm(1, 2), "mp3");
    player.play("", "pcm16");
    expect(started).toEqual([]);
    expect(player.busy).toBe(false);
  });

  test("stop silences everything queued and starts the next reply afresh", () => {
    const { output, started, sources } = fakeOutput();
    const player = new Player(() => output as AudioOutput);
    player.play(pcm(1, 2, 3, 4), "pcm16");
    player.play(pcm(5, 6), "pcm16");
    output.currentTime = 0.05;
    player.stop();
    expect(sources.every((source) => source.stopped)).toBe(true);
    expect(player.busy).toBe(false);
    player.play(pcm(7, 8), "pcm16");
    expect(started[2]?.start).toBe(0.05 + LEAD_SECS);
  });

  test("a buffer that ends leaves the queue, and close lets the context go", () => {
    const { output, sources } = fakeOutput();
    const player = new Player(() => output as AudioOutput);
    player.play(pcm(1, 2), "pcm16");
    sources[0]?.ended?.();
    expect(player.busy).toBe(false);
    player.close();
    expect(output.closed).toBe(true);
  });

  test("without Web Audio the player does nothing", () => {
    const player = new Player(() => null);
    player.unlock();
    player.play(pcm(1, 2), "pcm16");
    expect(player.busy).toBe(false);
    player.close();
  });

  test("starting one recording or reply stops the one playing", () => {
    const stopped: string[] = [];
    const first = () => stopped.push("first");
    const second = () => stopped.push("second");
    const releaseFirst = claim(first);
    claim(second);
    expect(stopped).toEqual(["first"]);
    // The first ended later on its own: the second still plays.
    releaseFirst();
    claim(first);
    expect(stopped).toEqual(["first", "second"]);
  });

  test("a spoken reply stops when a saved recording starts", () => {
    const { output, sources } = fakeOutput();
    const player = new Player(() => output as AudioOutput);
    player.play(pcm(1, 2), "pcm16");
    claim(() => undefined);
    expect(sources[0]?.stopped).toBe(true);
    expect(player.busy).toBe(false);
  });
});
