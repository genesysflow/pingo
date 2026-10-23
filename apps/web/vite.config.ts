import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
const backend = process.env.PINGO_BACKEND_URL ?? 'http://127.0.0.1:8090';
export default defineConfig({
  plugins: [svelte()],
  server: { proxy: {
    '/api': { target: backend, ws: true },
    '/rtc': { target: backend, ws: true },
  } },
  test: { include: ['tests/**/*.test.ts'] },
});
