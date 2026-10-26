/** DeepFilterNet's hop: 10 ms at 48 kHz. */
export const HOP = 480;

/**
 * Re-blocks Web Audio render quanta (128 samples) into model hops and back.
 * Output lags input by exactly one hop plus whatever `process` adds: the
 * prefilled hop of silence is the least that never underruns, because a hop
 * only completes after its last quantum arrives.
 */
export class HopFramer {
  private frame = new Float32Array(HOP);
  private filled = 0;
  private fifo: Float32Array;
  private read = 0;
  private size = HOP;
  constructor(capacity = HOP * 4) {
    this.fifo = new Float32Array(capacity);
  }
  /** `process` enhances each complete hop in place. A missing input is silence. */
  render(input: Float32Array | undefined, output: Float32Array, process: (frame: Float32Array) => void) {
    for (let i = 0; i < output.length; i++) {
      this.frame[this.filled++] = input?.[i] ?? 0;
      if (this.filled === HOP) {
        process(this.frame);
        this.push(this.frame);
        this.filled = 0;
      }
    }
    for (let i = 0; i < output.length; i++) {
      if (this.size === 0) {
        output[i] = 0;
        continue;
      }
      output[i] = this.fifo[this.read];
      this.read = (this.read + 1) % this.fifo.length;
      this.size--;
    }
  }
  private push(frame: Float32Array) {
    if (this.size + frame.length > this.fifo.length) throw new Error('HopFramer overflow');
    for (const sample of frame) {
      this.fifo[(this.read + this.size) % this.fifo.length] = sample;
      this.size++;
    }
  }
}
