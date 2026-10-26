import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { rmSync } from 'node:fs';
import { join } from 'node:path';
const backend = process.env.PINGO_BACKEND_URL ?? 'http://127.0.0.1:8090';
export default defineConfig({
  plugins: [
    svelte(),
    {
      // The desktop app suppresses noise natively; don't ship the browser model with it.
      name: 'pingo-desktop-without-browser-denoise',
      apply: 'build',
      writeBundle(options) {
        if (process.env.TAURI_ENV_PLATFORM && options.dir)
          rmSync(join(options.dir, 'denoise'), { recursive: true, force: true });
      },
    },
  ],
  server: { proxy: {
    '/api': { target: backend, ws: true },
    '/rtc': { target: backend, ws: true },
  } },
  test: { include: ['tests/**/*.test.ts'] },
});
