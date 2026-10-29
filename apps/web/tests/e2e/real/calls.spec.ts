import { test, expect, type Page } from '@playwright/test';
import { signup, createSpace, invite, accept } from './helpers';
import { fakeMicrophone, speech } from './fake-audio';

// Calls run with the default Gentle noise suppression, so the microphone must sound like a voice.
test.use({
  launchOptions: {
    args: [
      '--use-fake-ui-for-media-stream',
      '--use-fake-device-for-media-stream',
      `--use-file-for-fake-audio-capture=${fakeMicrophone('speech', 6, speech())}`,
      '--autoplay-policy=no-user-gesture-required',
      '--allow-loopback-in-peer-connection',
    ],
  },
});

declare global {
  interface Window {
    __peers: RTCPeerConnection[];
    __rtcSockets: WebSocket[];
  }
}
async function inbound(page: Page, kind: 'audio' | 'video') {
  return page.evaluate(async (kind) => {
    const peers = window.__peers;
    let bytes = 0;
    for (const pc of peers) {
      if (pc.connectionState === 'closed') continue;
      (await pc.getStats()).forEach((r) => {
        if (r.type === 'inbound-rtp' && r.kind === kind) bytes += r.bytesReceived ?? 0;
      });
    }
    return bytes;
  }, kind);
}
async function join(page: Page) {
  await page.getByRole('button', { name: 'Start a call', exact: false }).click();
  await expect(page.locator('.stage-tile')).toHaveCount(2, { timeout: 20_000 });
  await expect(page.getByText('Connecting…', { exact: true })).toHaveCount(0);
}
test('two browsers receive real audio, speaking/mute, screen video and SFU participant state', async ({
  browser,
}) => {
  const a = await browser.newContext({ permissions: ['microphone', 'camera'] });
  const b = await browser.newContext({ permissions: ['microphone', 'camera'] });
  for (const context of [a, b])
    await context.addInitScript((processRelay) => {
      localStorage.setItem('pingo-sound-checked', 'true');
      if (processRelay) localStorage.setItem('pingo-relay-only', 'true');
      const peers: RTCPeerConnection[] = [];
      window.__peers = peers;
      const sockets: WebSocket[] = [];
      window.__rtcSockets = sockets;
      const OriginalSocket = window.WebSocket;
      window.WebSocket = class extends OriginalSocket {
        constructor(url: string | URL, protocols?: string | string[]) {
          super(url, protocols);
          if (String(url).endsWith('/rtc')) sockets.push(this);
        }
      };
      const Original = window.RTCPeerConnection;
      window.RTCPeerConnection = class extends Original {
        constructor(config?: RTCConfiguration) {
          super(config);
          peers.push(this);
        }
      };
      // Browser capture dialogs cannot run headlessly. Exercise the production sender with real canvas video.
      navigator.mediaDevices.getDisplayMedia = async () => {
        const canvas = document.createElement('canvas');
        canvas.width = 1280;
        canvas.height = 720;
        const ctx = canvas.getContext('2d')!;
        const draw = () => {
          ctx.fillStyle = '#8064cc';
          ctx.fillRect(0, 0, 1280, 720);
          ctx.fillStyle = 'white';
          ctx.font = '48px sans-serif';
          ctx.fillText(`Pingo screen ${Date.now()}`, 70, 100);
        };
        draw();
        const timer = setInterval(draw, 50);
        const stream = canvas.captureStream(20);
        stream.getVideoTracks()[0].addEventListener('ended', () => clearInterval(timer));
        return stream;
      };
    }, process.env.PINGO_E2E_RELAY_ONLY === '1');
  const alice = await a.newPage(),
    bob = await b.newPage();
  const errors: string[] = [];
  for (const p of [alice, bob]) p.on('pageerror', (e) => errors.push(e.message));
  // Calls must run the DeepFilterNet worklet, not silently fall back to the browser's filter.
  for (const p of [alice, bob])
    p.on('console', (m) => {
      if (/DeepFilterNet|Noise suppression stopped/.test(m.text())) errors.push(m.text());
    });
  try {
    await signup(alice, 'CallAlice');
    await signup(bob, 'CallBob');
    await createSpace(alice);
    await accept(bob, await invite(alice));
    await alice.getByRole('button', { name: 'Start a call', exact: false }).click();
    await expect(alice.locator('.stage-tile')).toHaveCount(1);
    // Self-view must work even when there is nobody to forward camera media back.
    const selfCamera = alice.locator('video[aria-label="CallAlice\'s camera"]');
    await alice.getByRole('button', { name: 'Turn camera on', exact: true }).click();
    await expect(selfCamera).toBeVisible();
    await expect.poll(() => selfCamera.evaluate((node: HTMLVideoElement) => node.videoWidth)).toBe(1920);
    expect(await selfCamera.evaluate((node: HTMLVideoElement) => node.muted)).toBe(true);
    const firstCapture = await selfCamera.evaluateHandle((node: HTMLVideoElement) => node.srcObject as MediaStream);
    await alice.getByRole('button', { name: 'Turn camera off', exact: true }).click();
    await expect(selfCamera).toHaveCount(0);
    expect(await firstCapture.evaluate(stream => stream.getTracks().every(track => track.readyState === 'ended'))).toBe(true);
    await firstCapture.dispose();
    await join(bob);
    await expect(alice.locator('.stage-tile')).toHaveCount(2);
    await expect.poll(() => inbound(alice, 'audio'), { timeout: 25_000 }).toBeGreaterThan(0);
    await expect.poll(() => inbound(bob, 'audio'), { timeout: 25_000 }).toBeGreaterThan(0);
    if (process.env.PINGO_E2E_RELAY_ONLY === '1') {
      for (const page of [alice, bob]) {
        const relayed = await page.evaluate(async () => {
          const stats = await window.__peers.at(-1)!.getStats();
          const transport = [...stats.values()].find(r => r.type === 'transport' && r.selectedCandidatePairId);
          const pair = stats.get(transport.selectedCandidatePairId);
          return stats.get(pair.localCandidateId).candidateType;
        });
        expect(relayed).toBe('relay');
      }
    }
    // Speaking toggles with each syllable. Playwright's default visibility backoff
    // reaches one second and can keep sampling the gaps between them.
    await expect
      .poll(() => bob.locator('.stage-tile.speaking').filter({ hasText: 'CallAlice' }).isVisible(), {
        intervals: [50],
      })
      .toBe(true);
    await alice.getByRole('button', { name: 'Mute', exact: true }).click();
    await expect(bob.locator('.stage-tile.speaking').filter({ hasText: 'CallAlice' })).toHaveCount(0);
    let previous = await inbound(bob, 'audio'),
      quiet = 0;
    await expect
      .poll(
        async () => {
          const bytes = await inbound(bob, 'audio');
          quiet = bytes === previous ? quiet + 1 : 0;
          previous = bytes;
          return quiet;
        },
        { intervals: [250, 250, 250] },
      )
      .toBeGreaterThanOrEqual(3);
    await alice.getByRole('button', { name: 'Unmute', exact: true }).click();
    await expect.poll(() => inbound(bob, 'audio')).toBeGreaterThan(previous);
    await alice.getByRole('button', { name: 'Raise hand', exact: true }).click();
    await expect(bob.locator('.stage-tile').filter({ hasText: 'CallAlice' })).toContainText('✋');
    await alice.getByRole('button', { name: 'Send ❤️ in call' }).click();
    await expect(bob.locator('.floating-reaction')).toContainText('❤️');
    await alice.getByRole('button', { name: 'Turn camera on', exact: true }).click();
    await expect(selfCamera).toBeVisible();
    await expect.poll(() => selfCamera.evaluate((node: HTMLVideoElement) => node.videoWidth)).toBeGreaterThan(0);
    const camera = bob.locator('video[aria-label="CallAlice\'s camera"]');
    await expect(camera).toBeVisible();
    await expect.poll(() => camera.evaluate((node: HTMLVideoElement) => node.videoWidth), { timeout: 25_000 }).toBeGreaterThanOrEqual(960);
    const blur = alice.getByRole('button', { name: 'Blur camera background', exact: true });
    await blur.click();
    await expect(blur).toHaveAttribute('aria-pressed', 'true', { timeout: 25_000 });
    await expect.poll(() => selfCamera.evaluate((node: HTMLVideoElement) => node.videoHeight)).toBeLessThanOrEqual(720);
    const blurredBytes = await inbound(bob, 'video');
    await expect.poll(() => inbound(bob, 'video')).toBeGreaterThan(blurredBytes);
    await blur.click();
    await expect(blur).toHaveAttribute('aria-pressed', 'false');
    await expect.poll(() => selfCamera.evaluate((node: HTMLVideoElement) => node.videoWidth)).toBeGreaterThanOrEqual(960);
    await alice.getByRole('button', { name: 'Turn camera off', exact: true }).click();
    await expect(selfCamera).toHaveCount(0);
    await expect(camera).toHaveCount(0);
    await expect(alice.getByRole('button', { name: '2 people in general call' })).toBeVisible();
    await alice.getByRole('button', { name: 'Share your screen', exact: true }).click();
    const video = bob.locator('video[aria-label="Shared screen"]');
    await expect(video).toBeVisible({ timeout: 20_000 });
    await expect.poll(() => inbound(bob, 'video'), { timeout: 20_000 }).toBeGreaterThan(0);
    // Receiving bytes alone used to pass while the SFU stayed on the half-size layer.
    await expect.poll(() => video.evaluate((node: HTMLVideoElement) => node.videoWidth), { timeout: 25_000 }).toBe(1280);
    await bob.locator('.shared-screen').focus();
    await bob.keyboard.press('Enter');
    await expect(alice.locator('.share-pointer')).toBeVisible();
    await bob.keyboard.press('d');
    await expect(alice.locator('.annotation-stroke').first()).toBeVisible();
    await bob.screenshot({ path: 'screenshots/real-call-share.png', animations: 'disabled' });
    if (process.env.PINGO_E2E_RELAY_ONLY === '1') {
      // Credential renewal uses the same reconnect path. Keep the capture alive through replacement.
      const before = await alice.evaluate(() => window.__peers.length);
      await alice.evaluate(() => window.__rtcSockets.at(-1)!.close());
      await expect.poll(() => alice.evaluate(() => window.__peers.length), { timeout: 20_000 }).toBeGreaterThan(before);
      await expect(alice.getByRole('button', { name: 'Stop sharing', exact: true })).toBeVisible();
      await expect(video).toBeVisible({ timeout: 20_000 });
      await expect.poll(() => inbound(bob, 'video'), { timeout: 20_000 }).toBeGreaterThan(0);
    }
    await alice.getByRole('button', { name: 'Stop sharing', exact: true }).click();
    await expect(video).toHaveCount(0);
    const peersBefore = await bob.evaluate(() => window.__peers.length);
    await bob.evaluate(() => window.__rtcSockets.at(-1)!.close());
    await expect
      .poll(() => bob.evaluate(() => window.__peers.length), { timeout: 20_000 })
      .toBeGreaterThan(peersBefore);
    await expect.poll(() => inbound(bob, 'audio'), { timeout: 20_000 }).toBeGreaterThan(0);
    await expect(alice.locator('.stage-tile')).toHaveCount(2);
    await alice.getByRole('button', { name: 'Turn camera on', exact: true }).click();
    await expect(selfCamera).toBeVisible();
    await expect(camera).toBeVisible();
    const capture = await selfCamera.evaluateHandle((node: HTMLVideoElement) => node.srcObject as MediaStream);
    const alicePeersBefore = await alice.evaluate(() => window.__peers.length);
    await alice.evaluate(() => window.__rtcSockets.at(-1)!.close());
    await expect.poll(() => alice.evaluate(() => window.__peers.length), { timeout: 20_000 }).toBeGreaterThan(alicePeersBefore);
    await expect(selfCamera).toBeVisible();
    expect(await selfCamera.evaluate((node: HTMLVideoElement, stream) => node.srcObject === stream, capture)).toBe(true);
    await expect(camera).toBeVisible();
    await expect.poll(() => camera.evaluate((node: HTMLVideoElement) => node.videoWidth)).toBeGreaterThan(0);
    await alice.getByRole('button', { name: 'Leave this call', exact: true }).click();
    await expect(selfCamera).toHaveCount(0);
    expect(await capture.evaluate(stream => stream.getTracks().every(track => track.readyState === 'ended'))).toBe(true);
    await capture.dispose();
    await expect(bob.locator('.stage-tile')).toHaveCount(1);
    expect(errors).toEqual([]);
  } catch (error) {
    for (const p of [alice, bob])
      console.log(
        JSON.stringify(
          await p.evaluate(async () =>
            Promise.all(
              (window.__peers ?? []).map(async (pc) => ({
                connection: pc.connectionState,
                ice: pc.iceConnectionState,
                signaling: pc.signalingState,
                stats: [...(await pc.getStats()).values()].filter((r) =>
                  ['candidate-pair', 'transport'].includes(r.type),
                ),
              })),
            ),
          ),
        ),
      );
    throw error;
  } finally {
    await a.close();
    await b.close();
  }
});
