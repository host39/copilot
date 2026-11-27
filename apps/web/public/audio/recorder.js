// The push to talk recorder's AudioWorklet (src/audio/recorder.ts loads it). It mixes the
// microphone down to mono and posts the samples to the page in chunks of about 85 ms at 48 kHz;
// the page resamples them to 16 kHz and encodes the WAV file. A file rather than a module built
// into the bundle, so the strict Content Security Policy (script-src 'self') allows it.
// "stop" posts what is left, then "end".

const CHUNK_FRAMES = 4096;

class CopilotRecorder extends AudioWorkletProcessor {
  constructor() {
    super();
    this.chunk = new Float32Array(CHUNK_FRAMES);
    this.filled = 0;
    this.running = true;
    this.port.onmessage = (event) => {
      if (event.data !== "stop" || !this.running) return;
      this.flush();
      this.running = false;
      this.port.postMessage({ type: "end" });
    };
  }

  flush() {
    if (this.filled === 0) return;
    const samples = this.chunk.slice(0, this.filled);
    this.port.postMessage({ type: "samples", samples }, [samples.buffer]);
    this.filled = 0;
  }

  process(inputs) {
    if (!this.running) return false;
    const channels = inputs[0] ?? [];
    const frames = channels[0]?.length ?? 0;
    for (let frame = 0; frame < frames; frame += 1) {
      let sum = 0;
      for (const channel of channels) sum += channel[frame];
      this.chunk[this.filled] = sum / channels.length;
      this.filled += 1;
      if (this.filled === CHUNK_FRAMES) this.flush();
    }
    return true;
  }
}

registerProcessor("copilot-recorder", CopilotRecorder);
