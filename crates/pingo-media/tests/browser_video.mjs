// Run after `cargo build -p pingo-server -p pingo-media --example screenshare`
// and `cd apps/web && bun install`. Pass --vp8 with a vp8-enabled example,
// or --hardware to exercise VAAPI (requires a vaapi-enabled example/device).
// Uses the same Chromium Playwright dependency as the web suite; no UI files change.
import { createRequire } from 'node:module';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir, networkInterfaces } from 'node:os';
import { createServer } from 'node:net';
import { fileURLToPath } from 'node:url';
import { resolve, join } from 'node:path';
import assert from 'node:assert/strict';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const { chromium } = createRequire(resolve(root, 'apps/web/package.json'))('@playwright/test');
const dir = await mkdtemp(join(tmpdir(), 'pingo-native-browser-'));
const reservation = createServer();
await new Promise((resolve) => reservation.listen(0, '127.0.0.1', resolve));
const port = reservation.address().port;
await new Promise((resolve) => reservation.close(resolve));
const base = `http://127.0.0.1:${port}`;
const address = Object.values(networkInterfaces()).flat()
  .find((i) => i?.family === 'IPv4' && !i.internal)?.address ?? '127.0.0.1';
let backendLog = '';
const backend = spawn(process.env.PINGO_SERVER ?? resolve(root, 'target/debug/pingo-server'), [
  'serve', '--dev', '--http', `127.0.0.1:${port}`, '--dir', dir,
  '--codegen-out', join(dir, 'codegen'),
], { env: { ...process.env, PINGO_PUBLIC_IPS: address, PINGO_UDP_PORT: '0' }, stdio: ['ignore', 'pipe', 'pipe'] });
backend.stdout.on('data', (b) => { backendLog += b; });
backend.stderr.on('data', (b) => { backendLog += b; });
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const exited = (child) => new Promise((resolve) => {
  if (child.exitCode !== null || child.signalCode !== null) resolve(child.exitCode);
  else child.once('exit', resolve);
});
async function stop(child, signal = 'SIGTERM') {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const done = exited(child);
  child.kill(signal);
  const timer = setTimeout(() => child.kill('SIGKILL'), 5000);
  try { await done; } finally { clearTimeout(timer); }
}
async function post(path, body, access) {
  const response = await fetch(base + path, {
    method: 'POST', headers: { 'content-type': 'application/json', ...(access ? { authorization: `Bearer ${access}` } : {}) },
    body: JSON.stringify(body),
  });
  assert(response.ok, `${path}: HTTP ${response.status}`);
  return response.json();
}
async function call(kind, path, args, access) {
  const reply = await post(`/api/${kind}`, { path, args }, access);
  assert.equal(reply.status, 'success', JSON.stringify(reply));
  return reply.value;
}
async function login(email) {
  await post('/api/auth/magic/request', { email });
  const code = await call('action', 'dev:last_code', { email });
  return (await post('/api/auth/magic/verify', { email, code })).accessToken;
}
let browser, publisher;
try {
  let ready = false;
  for (let i = 0; i < 200; i++) {
    assert(backend.exitCode === null, backendLog);
    if (await fetch(`${base}/api/health`).then((r) => r.ok).catch(() => false)) { ready = true; break; }
    await delay(100);
  }
  assert(ready, `Backend startup timed out: ${backendLog}`);
  const space = await call('mutation', 'dev:seed', {});
  const alice = await login('alex@pingo.test'), bob = await login('sam@pingo.test');
  const rooms = await call('query', 'rooms:list', { space }, alice);
  const room = rooms.find((r) => r.name === 'Lounge')._id;
  browser = await chromium.launch({ headless: true, args: [
    '--autoplay-policy=no-user-gesture-required', '--allow-loopback-in-peer-connection',
  ] });
  for (const motion of [false, true]) {
    const admission = async (access) => (await call('mutation', 'voice:join', { room }, access)).token;
    const nativeToken = await admission(alice), viewerToken = await admission(bob);
    const page = await browser.newPage();
    await page.goto(`${base}/api/health`);
    await page.setContent('<video autoplay muted playsinline></video>');
    await page.evaluate(({ token, base }) => {
      const pc = window.peer = new RTCPeerConnection();
      const ws = window.socket = new WebSocket(base.replace('http:', 'ws:') + '/rtc');
      window.rtcErrors = [];
      pc.createDataChannel('pingo');
      pc.ontrack = ({ track }) => {
        if (track.kind !== 'video') return;
        const video = document.querySelector('video');
        video.srcObject = new MediaStream([track]);
        video.play().catch((e) => window.rtcErrors.push(String(e)));
      };
      const send = (message) => ws.send(JSON.stringify(message));
      const gather = async () => {
        if (pc.iceGatheringState === 'complete') return;
        await new Promise((resolve) => pc.addEventListener('icegatheringstatechange', () => {
          if (pc.iceGatheringState === 'complete') resolve();
        }));
      };
      let queue = Promise.resolve();
      ws.onopen = () => send({ type: 'join', token });
      ws.onmessage = ({ data }) => {
        queue = queue.then(async () => {
          const message = JSON.parse(data);
          if (message.type === 'joined') {
            await pc.setLocalDescription(await pc.createOffer()); await gather();
            send({ type: 'offer', sdp: pc.localDescription.sdp });
            send({ type: 'publishTracks', tracks: [] });
          } else if (message.type === 'answer') {
            await pc.setRemoteDescription({ type: 'answer', sdp: message.sdp });
          } else if (message.type === 'offer') {
            if (pc.signalingState !== 'stable') return;
            await pc.setRemoteDescription({ type: 'offer', sdp: message.sdp });
            await pc.setLocalDescription(await pc.createAnswer()); await gather();
            send({ type: 'answer', sdp: pc.localDescription.sdp });
          } else if (message.type === 'trackPublished') {
            send({ type: 'setVideoQuality', mid: message.track.mid, maxHeight: 720 });
          } else if (message.type === 'error') {
            window.rtcErrors.push(`${message.code}: ${message.message}`);
          }
        }).catch((e) => window.rtcErrors.push(String(e)));
      };
    }, { token: viewerToken, base });
    await page.waitForFunction(() => window.peer.connectionState === 'connected', null, { timeout: 15000 });
    let nativeLog = '';
    const vp8 = process.argv.includes('--vp8');
    publisher = spawn(process.env.PINGO_NATIVE_PUBLISHER ?? resolve(root, 'target/debug/examples/screenshare'), [
      '--url', `${base.replace('http:', 'ws:')}/rtc`, '--synthetic', '--seconds', '25',
      ...(process.argv.includes('--hardware') ? [] : ['--software']),
      ...(motion ? ['--motion'] : []), ...(vp8 ? ['--vp8'] : []),
    ], { env: { ...process.env, PINGO_MEDIA_TOKEN: nativeToken }, stdio: ['ignore', 'pipe', 'pipe'] });
    publisher.stdout.on('data', (b) => { nativeLog += b; });
    publisher.stderr.on('data', (b) => { nativeLog += b; });
    try {
      let decoded;
      for (let attempt = 0; attempt < 100; attempt++) {
        decoded = await page.evaluate(async () => {
        const stats = await window.peer.getStats();
        const inbound = [...stats.values()].find((r) => r.type === 'inbound-rtp' && r.kind === 'video');
        const video = document.querySelector('video');
        return { codec: inbound && stats.get(inbound.codecId)?.mimeType, frames: inbound?.framesDecoded ?? 0,
          width: video.videoWidth, height: video.videoHeight, fps: inbound?.framesPerSecond, errors: window.rtcErrors,
          connection: window.peer.connectionState };
        });
        if (decoded.frames >= 20 && decoded.width > 0) break;
        assert(publisher.exitCode === null, nativeLog);
        await delay(200);
      }
      assert(decoded.frames >= 20, JSON.stringify(decoded));
      assert.equal(decoded.codec, vp8 ? 'video/VP8' : 'video/H264');
      assert.deepEqual(decoded.errors, []);
      assert([320, 640].includes(decoded.width), JSON.stringify(decoded));
      assert.equal(decoded.height, decoded.width * 9 / 16);
      console.log(JSON.stringify({ mode: motion ? 'Motion' : 'Text', ...decoded }));
    } catch (e) {
      console.error(nativeLog);
      throw e;
    } finally {
      await stop(publisher, 'SIGINT'); publisher = undefined;
      await page.close();
    }
  }
} finally {
  await stop(publisher, 'SIGINT');
  await browser?.close();
  await stop(backend);
  await rm(dir, { recursive: true, force: true });
}
