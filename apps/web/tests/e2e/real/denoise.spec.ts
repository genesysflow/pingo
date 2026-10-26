import { test, expect } from '@playwright/test';
import { fakeMicrophone, whiteNoise } from './fake-audio';

// Chromium's fake microphone plays steady white noise at about -26 dBFS.
const wav = fakeMicrophone('noise', 4, whiteNoise());
test.use({
  launchOptions: {
    args: [
      '--use-fake-ui-for-media-stream',
      '--use-fake-device-for-media-stream',
      `--use-file-for-fake-audio-capture=${wav}`,
      '--autoplay-policy=no-user-gesture-required',
    ],
  },
});

test('the browser microphone chain runs DeepFilterNet on the captured audio', async ({ page }) => {
  test.skip(process.env.PINGO_E2E_PRODUCTION === '1', 'imports app source through the Vite dev server');
  await page.goto('/');
  const levels = await page.evaluate(async () => {
    // Served by Vite in the real suite; typed from the same module.
    const path = '/src/lib/call/microphone.ts';
    const { Microphone } = (await import(
      /* @vite-ignore */ path
    )) as typeof import('../../../src/lib/call/microphone');
    const measure = async (suppression: 'Off' | 'Gentle' | 'Strong') => {
      const microphone = await Microphone.open({ microphone: 'default', suppression, studio: false });
      const context = new AudioContext();
      const analyser = context.createAnalyser();
      analyser.fftSize = 2048;
      context.createMediaStreamSource(microphone.stream).connect(analyser);
      const data = new Float32Array(analyser.fftSize);
      // Let the model's running normalization settle, then average 1.5 s.
      await new Promise((r) => setTimeout(r, 1500));
      let power = 0,
        blocks = 0;
      const until = performance.now() + 1500;
      while (performance.now() < until) {
        analyser.getFloatTimeDomainData(data);
        power += data.reduce((sum, v) => sum + v * v, 0) / data.length;
        blocks++;
        await new Promise((r) => setTimeout(r, 40));
      }
      const result = {
        engine: microphone.engine,
        delay: microphone.delay,
        db: 10 * Math.log10(power / blocks),
      };
      microphone.stop();
      await context.close();
      return result;
    };
    return { off: await measure('Off'), gentle: await measure('Gentle'), strong: await measure('Strong') };
  });
  console.log(JSON.stringify(levels));
  expect(levels.off.engine).toBe('Off');
  expect(levels.gentle.engine).toBe('DeepFilterNet');
  expect(levels.strong.engine).toBe('DeepFilterNet');
  expect(levels.gentle.delay).toBe(1920);
  const gentle = levels.off.db - levels.gentle.db,
    strong = levels.off.db - levels.strong.db;
  // Gentle is capped at 18 dB of attenuation; the browser's AGC moves levels a little between runs.
  expect(gentle).toBeGreaterThan(10);
  expect(gentle).toBeLessThan(24);
  expect(strong).toBeGreaterThan(gentle + 15);
});
