/** Run the video spike inside the actual bundled WebKitGTK webview. */
import { mkdtemp, mkdir, rm } from 'node:fs/promises';
import { tmpdir, cpus, release } from 'node:os';
import { resolve, join } from 'node:path';
const desktop = resolve(import.meta.dirname, '..');
const root = resolve(desktop, '../..');
const work = await mkdtemp(join(tmpdir(), 'pingo-video-probe-'));
let passed = false;
const report = resolve(process.env.PINGO_DESKTOP_REPORT ?? join(desktop, 'benchmarks/webkit-video.json'));
const webkit = Bun.spawnSync(['pkg-config', '--modversion', 'webkit2gtk-4.1']);
const environment = { measuredAt: new Date().toISOString(), cpu: cpus()[0]?.model, kernel: release(),
  gdkBackend: process.env.GDK_BACKEND ?? 'default', waylandDisplay: process.env.WAYLAND_DISPLAY,
  disableDmabufRenderer: process.env.WEBKIT_DISABLE_DMABUF_RENDERER ?? '0', build: 'debug',
  webKitGTK: webkit.success ? webkit.stdout.toString().trim() : 'unknown' };
async function run(cmd: string[], cwd = desktop) {
  if (await Bun.spawn(cmd, { cwd, stdout: 'inherit', stderr: 'inherit' }).exited) throw new Error(`Failed: ${cmd.join(' ')}`);
}
try {
  if (process.env.PINGO_SKIP_BUILD !== '1') await run(['bun', 'run', 'tauri', 'build', '--debug', '--no-bundle']);
  const fixtures = process.env.PINGO_VIDEO_FIXTURES ?? join(work, 'fixtures');
  if (!process.env.PINGO_VIDEO_FIXTURES) await run(['sh', 'scripts/video-fixtures.sh', fixtures]);
  await mkdir(resolve(report, '..'), { recursive: true });
  await rm(report, { force: true });
  const child = Bun.spawn([join(root, 'target/debug/pingo')], {
    cwd: root, env: { ...process.env, XDG_CONFIG_HOME: join(work, 'config'), XDG_DATA_HOME: join(work, 'data'),
      PINGO_DESKTOP_PROBE: '1', PINGO_VIDEO_FIXTURES: fixtures, PINGO_DESKTOP_REPORT: report },
    stdout: Bun.file(join(work, 'app.log')), stderr: Bun.file(join(work, 'app.log')),
  });
  const timeout = setTimeout(() => child.kill(), 90_000);
  try {
    const code = await child.exited;
    if (code !== 0) throw new Error(`Probe exited ${code}; logs: ${work}`);
    const result = await Bun.file(report).json();
    result.environment = environment;
    await Bun.write(report, JSON.stringify(result, null, 2) + '\n');
    if (result.error) throw new Error(`Probe: ${result.error}`);
    if (result.samples.some((sample: { displayed: number; errors: string[] }) => !sample.displayed || sample.errors.length)) throw new Error(`No displayed frames or display errors; logs: ${work}`);
    passed = true;
    console.log(JSON.stringify(result, null, 2));
  } catch (error) {
    if (!await Bun.file(report).exists()) {
      await Bun.write(report, JSON.stringify({ environment, error: String(error) }, null, 2) + '\n');
    }
    throw error;
  } finally { clearTimeout(timeout); child.kill(); }
} finally { if (passed) await rm(work, { recursive: true, force: true }); else console.error(`Probe logs retained at ${work}`); }
