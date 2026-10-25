/** Real WebKit/Tauri UI + real RxFerrix/SFU + a synthetic native Rust peer. No media mocks. */
import { mkdtemp, rm, mkdir, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { createServer } from 'node:net';
import { createRequire } from 'node:module';

const root = resolve(import.meta.dirname, '../../..');
const artifacts = resolve(root, 'apps/desktop/screenshots');
const work = await mkdtemp(join(tmpdir(), 'pingo-desktop-smoke-'));
const children: ReturnType<typeof Bun.spawn>[] = [];
let session = '', driverOrigin = '', passed = false;
let browser: { close(): Promise<void> } | undefined;
async function port() {
  const listener = createServer();
  await new Promise<void>(r => listener.listen(0, '127.0.0.1', r));
  const number = (listener.address() as { port: number }).port;
  await new Promise<void>(r => listener.close(() => r()));
  return number;
}
async function run(cmd: string[], cwd = root) {
  const child = Bun.spawn(cmd, { cwd, stdout: 'inherit', stderr: 'inherit' });
  if (await child.exited) throw new Error(`Command failed: ${cmd.join(' ')}`);
}
function start(cmd: string[], name: string, env: Record<string, string | undefined> = {}) {
  const log = Bun.file(join(work, `${name}.log`));
  const child = Bun.spawn(cmd, { cwd: root, env: { ...process.env, ...env }, stdout: log, stderr: log });
  children.push(child); return child;
}
async function until<T>(description: string, test: () => Promise<T>, seconds = 25): Promise<T> {
  const deadline = Date.now() + seconds * 1000;
  let last: unknown;
  while (Date.now() < deadline) {
    try { const result = await test(); if (result) return result; } catch (e) { last = e; }
    await Bun.sleep(100);
  }
  throw new Error(`Timed out: ${description}${last ? `: ${last}` : ''}`);
}
async function webdriver(path: string, body?: unknown, method = body === undefined ? 'GET' : 'POST') {
  const response = await fetch(`${driverOrigin}${path}`, {
    method, headers: { 'Content-Type': 'application/json' }, ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    signal: AbortSignal.timeout(30_000),
  });
  const { value } = await response.json() as { value: any };
  if (!response.ok || value?.error) throw new Error(JSON.stringify(value));
  return value;
}
const command = (path: string, body?: unknown) => webdriver(`/session/${session}${path}`, body);
const evaluate = (script: string, args: unknown[] = []) => command('/execute/sync', { script, args });
async function element(script: string, args: unknown[] = []) {
  const node = await until('UI element', () => evaluate(script, args));
  return node['element-6066-11e4-a52e-4f735466cecf'];
}
async function fill(label: string, value: string) {
  const id = await element('return [...document.querySelectorAll("label")].find(e=>e.textContent.trim().startsWith(arguments[0]))?.querySelector("input")', [label]);
  await command(`/element/${id}/clear`, {});
  await command(`/element/${id}/value`, { text: value });
}
async function click(text: string) {
  const id = await element('return [...document.querySelectorAll("button,a")].find(e=>e.textContent.trim()===arguments[0] || e.getAttribute("aria-label")===arguments[0])', [text]);
  await command(`/element/${id}/click`, {});
}
async function api(origin: string, path: string, body: unknown, auth?: string) {
  const response = await fetch(`${origin}/api/${path}`, {
    method: 'POST', headers: { 'Content-Type': 'application/json', ...(auth ? { Authorization: `Bearer ${auth}` } : {}) },
    body: JSON.stringify(body), signal: AbortSignal.timeout(10_000),
  });
  const data = await response.json() as any;
  if (!response.ok) throw new Error(`${path}: ${JSON.stringify(data)}`);
  return data;
}
async function fn(origin: string, kind: string, path: string, args: object, auth: string) {
  const data = await api(origin, kind, { path, args }, auth);
  if (data.status !== 'success') throw new Error(JSON.stringify(data));
  return data.value;
}
try {
  if (process.env.PINGO_SKIP_BUILD !== '1') {
    await run(['bun', 'run', 'tauri', 'build', '--debug', '--no-bundle'], join(root, 'apps/desktop'));
    await run(['cargo', 'build', '-p', 'pingo-server']);
    await run(['cargo', 'build', '-p', 'pingo-media', '--example', 'join']);
  }
  const httpPort = await port(), driverPort = await port();
  const origin = `http://127.0.0.1:${httpPort}`;
  start([resolve(process.env.PINGO_SERVER ?? join(root, 'target/debug/pingo-server')), 'serve', '--dev', '--http', `127.0.0.1:${httpPort}`, '--dir', join(work, 'server'), '--codegen-out', join(work, 'codegen')], 'server', { PINGO_PUBLIC_IPS: '127.0.0.1', PINGO_UDP_PORT: '0', PINGO_APP_URL: origin, PINGO_WEB_DIR: join(root, 'apps/web/dist') });
  await until('development server', async () => (await fetch(`${origin}/api/health`)).ok);
  driverOrigin = `http://127.0.0.1:${driverPort}`;
  start(['WebKitWebDriver', `--port=${driverPort}`], 'webdriver', {
    TAURI_WEBVIEW_AUTOMATION: 'true', PINGO_DESKTOP_TEST_HEADLESS: '1',
    ALSA_CONFIG_PATH: process.env.PINGO_DESKTOP_ALSA_CONFIG ?? process.env.ALSA_CONFIG_PATH,
    XDG_CONFIG_HOME: join(work, 'config'), XDG_DATA_HOME: join(work, 'data'), XDG_CACHE_HOME: join(work, 'cache'),
  });
  await until('WebKitWebDriver', () => webdriver('/status'));
  const created = await webdriver('/session', { capabilities: { alwaysMatch: {
    browserName: 'wry', 'webkitgtk:browserOptions': { binary: resolve(process.env.PINGO_DESKTOP_BINARY ?? join(root, 'target/debug/pingo')) },
  } } });
  session = created.sessionId;
  await fill('Server URL', origin); await click('Continue');
  await click('Create an account');
  await fill('Your name', 'Desktop Smoke');
  await fill('Email address', 'desktop@pingo.test'); await fill('Password', 'correct-horse-pingo');
  await click('Make yourself at home');
  await fill('Space name', 'Native smoke'); await click('Create space');
  await until('general room', () => evaluate('return !!document.querySelector(".room-heading") || [...document.querySelectorAll("h1,h2")].some(e=>e.textContent.trim()==="general")'));
  const auth = await evaluate('return JSON.parse(localStorage.getItem(arguments[0])).accessToken', [`rxferrix.auth:${origin}`]);
  const spaces = await fn(origin, 'query', 'spaces:list', {}, auth);
  const space = spaces[0]._id;
  const rooms = await fn(origin, 'query', 'rooms:list', { space }, auth);
  const room = rooms.find((r: any) => r.name === 'general')._id;
  const peer = await api(origin, 'auth/signup', { name: 'Native Peer', email: 'peer@pingo.test', password: 'correct-horse-pingo' });
  const invite = await fn(origin, 'mutation', 'invites:create', { space }, auth);
  await fn(origin, 'mutation', 'invites:accept', { code: invite }, peer.accessToken);
  const admission = await fn(origin, 'mutation', 'voice:join', { room }, peer.accessToken);
  start([join(root, 'target/debug/examples/join'), '--url', `${origin.replace('http:', 'ws:')}/rtc`, '--headless', '--seconds', '40'], 'peer', { PINGO_MEDIA_TOKEN: admission.token });
  await evaluate(`localStorage.setItem('pingo-sound-checked','true');
    window.__nativeSmoke={packets:0,pcmBytes:0,events:[]};
    const core=window.__TAURI_INTERNALS__;
    core.invoke('plugin:event|listen',{event:'native-call',target:{kind:'Any'},handler:core.transformCallback(({payload})=>{
      if(['trackPublished','trackUnpublished','error','deviceError','connected'].includes(payload.event.type)) window.__nativeSmoke.events.push(payload.event);
      if(payload.event.type==='playout') window.__nativeSmoke.packets=payload.event.receivedPackets;
      if(payload.event.type==='playback') window.__nativeSmoke.pcmBytes=payload.event.receivedPcmBytes;
    })}); return true;`);
  const joinButton = await element('return [...document.querySelectorAll("button")].find(e=>/^Start a call|people in general call/.test(e.getAttribute("aria-label")??e.textContent.trim()))');
  await command(`/element/${joinButton}/click`, {});
  const result = await until('native remote PCM and two UI participants', () => evaluate(`
    const state=window.__nativeSmoke;
    const tiles=[...document.querySelectorAll('.stage-tile')].map(e=>e.textContent);
    return state.packets>0 && state.pcmBytes>0 && tiles.length===2 ? {...state,participants:tiles} : null;`));
  const participants = await fn(origin, 'query', 'voice:participants', { room }, auth);
  if (participants.length !== 2) throw new Error('Expected two durable voice participants');
  await mkdir(artifacts, { recursive: true });
  const screenshot = await command('/screenshot');
  await writeFile(join(artifacts, 'native-call.png'), Buffer.from(screenshot, 'base64'));
  if (process.env.PINGO_SMOKE_VIDEO === '1') {
    const { chromium } = createRequire(join(root, 'apps/web/package.json'))('@playwright/test');
    browser = await chromium.launch({ args: ['--mute-audio', '--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream', '--allow-loopback-in-peer-connection', '--autoplay-policy=no-user-gesture-required'] });
    const page = await (browser as any).newPage();
    page.on('pageerror', (error: Error) => console.error('Screen publisher page error:', error.message));
    page.on('console', (message: { type(): string; text(): string }) => {
      if (message.type() === 'error') console.error('Screen publisher:', message.text());
    });
    await page.addInitScript(() => {
      localStorage.setItem('pingo-sound-checked', 'true');
      navigator.mediaDevices.getDisplayMedia = async () => {
        const canvas = document.createElement('canvas'); canvas.width = 1280; canvas.height = 720;
        const draw = () => { const context = canvas.getContext('2d')!; context.fillStyle = '#8064cc'; context.fillRect(0, 0, 1280, 720); context.fillStyle = 'white'; context.font = '48px sans-serif'; context.fillText(`Native screen display ${Date.now()}`, 70, 100); };
        draw(); setInterval(draw, 50); return canvas.captureStream(20);
      };
    });
    await page.goto(`${origin}/signup`);
    await page.getByLabel('Your name', { exact: true }).fill('Screen Peer');
    await page.getByLabel('Email address', { exact: true }).fill('screen@pingo.test');
    await page.getByLabel('Password', { exact: true }).fill('correct-horse-pingo');
    await page.getByRole('button', { name: 'Make yourself at home', exact: true }).click();
    await page.getByRole('heading', { name: 'Make a little space.' }).waitFor();
    const screenAuth = await page.evaluate((key: string) => JSON.parse(localStorage.getItem(key)!).accessToken, `rxferrix.auth:${origin}`);
    await fn(origin, 'mutation', 'invites:accept', { code: invite }, screenAuth);
    await page.goto(origin + await evaluate('return location.pathname'));
    await page.getByRole('button', { name: 'Start a call', exact: true }).click();
    await page.waitForFunction(() => !!document.querySelector('.stage-view') && !document.querySelector('.stage-view > [role="status"]'));
    await page.getByRole('button', { name: 'Share your screen', exact: true }).click();
    result.video = await until('remote screen decoded into the native Stage canvas', () => evaluate(`
      const canvas=document.querySelector('canvas[aria-label="Shared screen"]');
      if(!canvas || canvas.width<640) return null;
      const pixel=[...canvas.getContext('2d').getImageData(10,10,1,1).data];
      return pixel[0]>30 && pixel[2]>30 ? {width:canvas.width,height:canvas.height,pixel} : null;`));
    const screen = await command('/screenshot');
    await writeFile(join(artifacts, 'native-screen.png'), Buffer.from(screen, 'base64'));
    await page.getByRole('button', { name: 'Stop sharing', exact: true }).click();
    await until('remote screen removed after unpublish', () => evaluate('return !document.querySelector(\'canvas[aria-label="Shared screen"]\')'));
  }
  await writeFile(join(artifacts, 'smoke.json'), JSON.stringify({ ...result, durableParticipants: participants.length }, null, 2) + '\n');
  console.log('Desktop UI smoke passed:', JSON.stringify(result));
  await click('Leave this call');
  passed = true;
} catch (error) {
  if (browser) {
    const publisher = (browser as any).contexts()[0]?.pages()[0];
    if (publisher) await writeFile(join(work, 'publisher.json'), JSON.stringify(await publisher.evaluate(() => document.body.innerText).catch(String)));
  }
  if (session) {
    const diagnostics = await evaluate('return {state:window.__nativeSmoke,canvas:[...document.querySelectorAll("canvas")].map(c=>({label:c.getAttribute("aria-label"),width:c.width,height:c.height})),text:document.body.innerText}').catch(String);
    await writeFile(join(work, 'diagnostics.json'), JSON.stringify(diagnostics, null, 2));
    const screenshot = await command('/screenshot').catch(() => null);
    if (screenshot) await writeFile(join(work, 'failure.png'), Buffer.from(screenshot, 'base64'));
  }
  console.error(`Smoke logs retained at ${work}`); throw error;
} finally {
  await browser?.close();
  if (session) await webdriver(`/session/${session}`, undefined, 'DELETE').catch(() => {});
  for (const child of children.reverse()) { child.kill(); await child.exited; }
  if (passed) await rm(work, { recursive: true, force: true });
}
