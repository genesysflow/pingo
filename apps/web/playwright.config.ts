import { defineConfig } from '@playwright/test';
const mock = process.env.VITE_PINGO_MOCK === '1';
const production = process.env.PINGO_E2E_PRODUCTION === '1';
const port = mock
  ? '5179'
  : production
    ? (process.env.PINGO_E2E_PORT ?? '8098')
    : (process.env.PINGO_E2E_WEB_PORT ?? '5188');
const baseURL = `http://127.0.0.1:${port}`;
export default defineConfig({
  outputDir: `test-results/${mock ? 'mock' : production ? 'production' : 'live'}`,
  testDir: mock ? './tests/e2e' : './tests/e2e/real',
  testIgnore: mock ? '**/real/**' : undefined,
  fullyParallel: false,
  workers: 1,
  timeout: 60_000,
  expect: { timeout: 12_000 },
  use: {
    baseURL,
    viewport: { width: 1440, height: 900 },
    reducedMotion: 'reduce',
    trace: 'retain-on-failure',
    launchOptions: {
      args: [
        '--use-fake-ui-for-media-stream',
        '--use-fake-device-for-media-stream',
        '--autoplay-policy=no-user-gesture-required',
        '--allow-loopback-in-peer-connection',
      ],
    },
  },
  webServer: {
    command: mock ? `bun run dev --port ${port} --strictPort` : 'bun tests/e2e/server.ts',
    url: production ? `${baseURL}/api/health` : baseURL,
    reuseExistingServer: false,
    timeout: 120_000,
  },
  reporter: 'list',
});
