import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

/** Writes 48 kHz mono 16-bit PCM for Chromium's `--use-file-for-fake-audio-capture` (it loops). */
export function fakeMicrophone(name: string, seconds: number, sample: (i: number) => number) {
  const rate = 48_000,
    samples = rate * seconds,
    data = Buffer.alloc(44 + samples * 2);
  data.write('RIFF', 0);
  data.writeUInt32LE(36 + samples * 2, 4);
  data.write('WAVEfmt ', 8);
  data.writeUInt32LE(16, 16);
  data.writeUInt16LE(1, 20);
  data.writeUInt16LE(1, 22);
  data.writeUInt32LE(rate, 24);
  data.writeUInt32LE(rate * 2, 28);
  data.writeUInt16LE(2, 32);
  data.writeUInt16LE(16, 34);
  data.write('data', 36);
  data.writeUInt32LE(samples * 2, 40);
  for (let i = 0; i < samples; i++)
    data.writeInt16LE(Math.round(Math.max(-1, Math.min(1, sample(i))) * 32767), 44 + i * 2);
  const path = join(mkdtempSync(join(tmpdir(), 'pingo-fake-mic-')), `${name}.wav`);
  writeFileSync(path, data);
  return path;
}

/** Steady white noise at about -26 dBFS RMS. */
export function whiteNoise() {
  let seed = 1;
  return (_: number) => {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    return (seed / 2 ** 32 - 0.5) * 0.17;
  };
}

/**
 * Voiced syllables (a glottal source with drifting pitch through two formants),
 * the generator DeepFilterNet is tested to pass in crates/pingo-denoise. Chromium's
 * default fake microphone is a pure beep, which noise suppression rightly removes.
 */
export function speech() {
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
  return (i: number) => {
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
        source * (1 - r) + 2 * r * Math.cos((2 * Math.PI * f) / 48_000) * state[k][0] - r * r * state[k][1];
      state[k] = [v, state[k][0]];
      y += v;
    });
    return 0.25 * envelope * y;
  };
}
