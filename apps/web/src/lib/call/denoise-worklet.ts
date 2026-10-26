// AudioWorklet that runs DeepFilterNet3 (crates/pingo-denoise, built to WASM)
// on the microphone before it is sent. Loaded with `audioWorklet.addModule`.
import { HOP, HopFramer } from './hop-framer';

declare const sampleRate: number;
declare function registerProcessor(name: string, processor: unknown): void;
declare class AudioWorkletProcessor {
  readonly port: MessagePort;
}

interface DenoiseExports {
  memory: WebAssembly.Memory;
  pingo_denoise_new(strong: number): number;
  pingo_denoise_free(handle: number): void;
  pingo_denoise_frame(handle: number): number;
  pingo_denoise_process(handle: number): number;
  pingo_denoise_set_strength(handle: number, strong: number): void;
  pingo_denoise_delay(handle: number): number;
}
export interface DenoiseOptions {
  module: WebAssembly.Module;
  strong: boolean;
}
export type DenoiseMessage = { type: 'ready'; delay: number } | { type: 'error'; message: string };

class DenoiseProcessor extends AudioWorkletProcessor {
  private wasm?: DenoiseExports;
  private handle = 0;
  private framer = new HopFramer();
  private closed = false;
  constructor(options: { processorOptions: DenoiseOptions }) {
    super();
    try {
      if (sampleRate !== 48_000) throw new Error(`DeepFilterNet needs 48 kHz, not ${sampleRate} Hz`);
      const { module, strong } = options.processorOptions;
      // Synchronous instantiation is fine off the main thread; this context only carries the mic.
      const wasm = new WebAssembly.Instance(module, {}).exports as unknown as DenoiseExports;
      this.handle = wasm.pingo_denoise_new(strong ? 1 : 0);
      if (!this.handle) throw new Error('the noise suppression model did not load');
      this.wasm = wasm;
      this.post({ type: 'ready', delay: wasm.pingo_denoise_delay(this.handle) + HOP });
    } catch (error) {
      this.post({ type: 'error', message: String(error) });
    }
    this.port.onmessage = ({ data }) => {
      if (data?.type === 'strength' && this.wasm)
        this.wasm.pingo_denoise_set_strength(this.handle, data.strong ? 1 : 0);
      if (data?.type === 'close') {
        this.closed = true;
        if (this.wasm) this.wasm.pingo_denoise_free(this.handle);
        this.wasm = undefined;
      }
    };
  }
  private post(message: DenoiseMessage) {
    this.port.postMessage(message);
  }
  process(inputs: Float32Array[][], outputs: Float32Array[][]) {
    const output = outputs[0]?.[0];
    if (!output) return !this.closed;
    if (!this.wasm) {
      // Failed or closed: keep the microphone flowing rather than going silent.
      const input = inputs[0]?.[0];
      if (input) output.set(input);
      return !this.closed;
    }
    this.framer.render(inputs[0]?.[0], output, (frame) => this.enhance(frame));
    return true;
  }
  private enhance(frame: Float32Array) {
    const wasm = this.wasm;
    if (!wasm) return;
    try {
      new Float32Array(wasm.memory.buffer, wasm.pingo_denoise_frame(this.handle), HOP).set(frame);
      const lsnr = wasm.pingo_denoise_process(this.handle);
      // Re-create the view: inference may have grown (and detached) the memory.
      if (!Number.isNaN(lsnr))
        frame.set(new Float32Array(wasm.memory.buffer, wasm.pingo_denoise_frame(this.handle), HOP));
    } catch (error) {
      // A trap leaves the instance unusable; pass audio through from now on.
      this.wasm = undefined;
      this.post({ type: 'error', message: String(error) });
    }
  }
}
registerProcessor('pingo-denoise', DenoiseProcessor);
