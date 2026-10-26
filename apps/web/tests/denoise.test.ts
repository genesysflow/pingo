import { describe, expect, it } from 'vitest';
import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { HOP, HopFramer } from '../src/lib/call/hop-framer';

describe('HopFramer', () => {
  it('delays by exactly one hop, never underruns, and processes whole hops', () => {
    for (const quantum of [128, 256, 441, 480, 1000]) {
      const framer = new HopFramer();
      const input = Float32Array.from({ length: quantum * 40 }, (_, i) => Math.sin(i * 0.01) + 2);
      const output = new Float32Array(input.length);
      let hops = 0;
      for (let at = 0; at < input.length; at += quantum) {
        const block = output.subarray(at, at + quantum);
        framer.render(input.subarray(at, at + quantum), block, (frame) => {
          expect(frame.length).toBe(HOP);
          hops++;
        });
      }
      expect(hops).toBe(Math.floor(input.length / HOP));
      expect(Array.from(output.subarray(0, HOP)).every((v) => v === 0)).toBe(true);
      // Every later sample is the input one hop earlier: no gaps, no repeats.
      for (let i = HOP; i < input.length; i++) expect(output[i]).toBe(input[i - HOP]);
    }
  });
  it('treats a disconnected input as silence', () => {
    const framer = new HopFramer();
    const output = new Float32Array(128).fill(9);
    for (let i = 0; i < 10; i++) framer.render(undefined, output, () => {});
    expect(Array.from(output).every((v) => v === 0)).toBe(true);
  });
});

const wasmPath = resolve(import.meta.dirname, '../public/denoise/pingo-denoise.wasm');
describe.skipIf(!existsSync(wasmPath))('DeepFilterNet WASM (bun run denoise)', () => {
  it('runs the browser build: Gentle caps attenuation, Strong removes noise, speech survives', async () => {
    const module = await WebAssembly.compile(readFileSync(wasmPath));
    const run = (strong: boolean, signal: (i: number) => number, seconds: number) => {
      const wasm = new WebAssembly.Instance(module, {}).exports as unknown as {
        memory: WebAssembly.Memory;
        pingo_denoise_new(strong: number): number;
        pingo_denoise_frame(handle: number): number;
        pingo_denoise_process(handle: number): number;
        pingo_denoise_delay(handle: number): number;
        pingo_denoise_free(handle: number): void;
      };
      const handle = wasm.pingo_denoise_new(strong ? 1 : 0);
      expect(handle).not.toBe(0);
      expect(wasm.pingo_denoise_delay(handle)).toBe(1440);
      const framer = new HopFramer();
      const input = Float32Array.from({ length: seconds * 48_000 }, (_, i) => signal(i));
      const output = new Float32Array(input.length);
      for (let at = 0; at < input.length; at += 128)
        framer.render(input.subarray(at, at + 128), output.subarray(at, at + 128), (frame) => {
          new Float32Array(wasm.memory.buffer, wasm.pingo_denoise_frame(handle), HOP).set(frame);
          expect(Number.isNaN(wasm.pingo_denoise_process(handle))).toBe(false);
          frame.set(new Float32Array(wasm.memory.buffer, wasm.pingo_denoise_frame(handle), HOP));
        });
      wasm.pingo_denoise_free(handle);
      return { input, output };
    };
    const energy = (x: Float32Array) => x.reduce((sum, v) => sum + v * v, 0);
    let seed = 7;
    const noise = () => {
      seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
      return (seed / 2 ** 32 - 0.5) * 0.1;
    };
    const reduction = (strong: boolean) => {
      const { input, output } = run(strong, noise, 3);
      return 10 * Math.log10(energy(input.subarray(48_000)) / energy(output.subarray(48_000)));
    };
    const gentle = reduction(false),
      strong = reduction(true);
    expect(gentle).toBeGreaterThan(12);
    expect(gentle).toBeLessThan(19);
    expect(strong).toBeGreaterThan(35);
    // Speech keeps its level (same generator as crates/pingo-denoise/tests): a
    // glottal source with drifting pitch through two formants, in syllables.
    const speech = (() => {
      const samples = new Float32Array(3 * 48_000);
      const formants = [
        [
          [700, 90],
          [1220, 110],
        ],
        [
          [300, 60],
          [2300, 150],
        ],
        [
          [500, 80],
          [900, 100],
        ],
      ];
      const state = [
        [0, 0],
        [0, 0],
      ];
      let phase = 0;
      for (let i = 0; i < samples.length; i++) {
        const t = i / 48_000;
        const pitch = 120 + 25 * Math.sin(2 * Math.PI * 0.7 * t) + 8 * Math.sin(2 * Math.PI * 5 * t);
        phase = (phase + pitch / 48_000) % 1;
        const source = (phase < 0.4 ? Math.sin((2 * Math.PI * phase) / 0.8) : 0) - 0.25;
        const syllable = (t * 3.2) % 1;
        const envelope = syllable < 0.75 ? Math.sin((Math.PI * syllable) / 0.75) : 0;
        let y = 0;
        formants[Math.floor(t * 1.6) % 3].forEach(([f, bw], k) => {
          const r = Math.exp((-Math.PI * bw) / 48_000);
          const v =
            source * (1 - r) +
            2 * r * Math.cos((2 * Math.PI * f) / 48_000) * state[k][0] -
            r * r * state[k][1];
          state[k] = [v, state[k][0]];
          y += v;
        });
        samples[i] = 0.25 * envelope * y;
      }
      return samples;
    })();
    const vowel = (i: number) => speech[i];
    const { input, output } = run(false, vowel, 3);
    const kept =
      10 * Math.log10(energy(output.subarray(48_000 + 1920)) / energy(input.subarray(48_000, -1920)));
    expect(Math.abs(kept)).toBeLessThan(3);
  }, 60_000);
});
