import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir, networkInterfaces } from 'node:os';
import { resolve, join } from 'node:path';
const root = resolve(import.meta.dirname, '../../../..');
const dir = await mkdtemp(join(tmpdir(), 'pingo-web-e2e-'));
const port = process.env.PINGO_E2E_PORT ?? '8098';
const webPort = process.env.PINGO_E2E_WEB_PORT ?? '5188';
const origin = `http://127.0.0.1:${port}`;
const production = process.env.PINGO_E2E_PRODUCTION === '1';
const mediaIp =
  process.env.PINGO_E2E_MEDIA_IP ??
  Object.values(networkInterfaces())
    .flat()
    .find((i) => i && i.family === 'IPv4' && !i.internal)?.address ?? '127.0.0.1';
const env = {
  ...process.env,
  PINGO_PUBLIC_IPS: mediaIp,
  PINGO_UDP_PORT: '0',
  PINGO_APP_URL: production ? origin : `http://127.0.0.1:${webPort}`,
  ...(production ? { PINGO_WEB_DIR: resolve(root, 'apps/web/dist') } : {}),
};
const backend = spawn(
  process.env.PINGO_SERVER ?? resolve(root, 'target/debug/pingo-server'),
  [
    'serve',
    '--dev',
    '--http',
    `127.0.0.1:${port}`,
    '--dir',
    dir,
    '--codegen-out',
    resolve(root, 'apps/web/src/lib/rxferrix'),
  ],
  { cwd: root, env, stdio: 'inherit' },
);
let vite: ReturnType<typeof spawn> | undefined;
let stopping = false;
async function stop() {
  if (stopping) return;
  stopping = true;
  vite?.kill('SIGTERM');
  backend.kill('SIGTERM');
  await new Promise((r) => setTimeout(r, 700));
  await rm(dir, { recursive: true, force: true });
  process.exit(0);
}
process.on('SIGTERM', () => void stop());
process.on('SIGINT', () => void stop());
backend.on('error', (e) => {
  console.error(e);
  void stop();
});
backend.on('exit', () => {
  if (!stopping) void stop();
});
for (let i = 0; i < 300; i++) {
  try {
    if ((await fetch(`${origin}/api/health`)).ok) break;
  } catch {
    /* starting */
  }
  if (i === 299) throw new Error('Backend did not start');
  await new Promise((r) => setTimeout(r, 100));
}
if (!production) {
  vite = spawn('bun', ['run', 'dev', '--port', webPort, '--strictPort'], {
    cwd: resolve(root, 'apps/web'),
    env: { ...env, PINGO_BACKEND_URL: origin, VITE_PINGO_MOCK: '0' },
    stdio: 'inherit',
  });
  vite.on('exit', () => {
    if (!stopping) void stop();
  });
}
