// The recorder's resampler: the microphone runs at the device's rate (often 44.1 or 48 kHz), and
// clips are 16 kHz. Each output sample is the average of the input it covers (a box filter, which
// also keeps most of what lies above the new Nyquist rate out of the clip), weighted where an
// output sample's span starts or ends inside an input sample, so any pair of rates works.
// Input arrives in chunks; what a chunk leaves over waits for the next one.

export class Resampler {
  /** Input samples per output sample. */
  private readonly ratio: number;
  /** Input not yet used up, from the sample the next output starts in. */
  private carry = new Float32Array(0);
  /** Where the next output starts in `carry`, as a fraction of its first sample (`0` to `1`). */
  private offset = 0;

  constructor(from: number, to: number) {
    if (!(from > 0 && to > 0)) throw new RangeError("Sample rates must be positive.");
    this.ratio = from / to;
  }

  /** The output samples `input` completes; samples a later chunk completes wait for it. */
  push(input: Float32Array): Float32Array {
    const buffer = new Float32Array(this.carry.length + input.length);
    buffer.set(this.carry);
    buffer.set(input, this.carry.length);

    const out: number[] = [];
    let start = this.offset;
    while (start + this.ratio <= buffer.length) {
      const end = start + this.ratio;
      let sum = 0;
      for (let index = Math.floor(start); index < Math.ceil(end); index += 1) {
        const weight = Math.min(end, index + 1) - Math.max(start, index);
        sum += (buffer[index] ?? 0) * weight;
      }
      out.push(sum / this.ratio);
      start = end;
    }

    const whole = Math.floor(start);
    this.carry = buffer.slice(whole);
    this.offset = start - whole;
    return Float32Array.from(out);
  }
}
