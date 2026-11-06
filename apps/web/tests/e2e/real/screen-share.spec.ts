import { test, expect } from '@playwright/test';
import { signup, createSpace, invite, accept } from './helpers';

test('screen annotations stay aligned and share controls reset between shares', async ({ browser }) => {
  const contexts = await Promise.all(
    [1440, 1000].map((width) =>
      browser.newContext({
        permissions: ['microphone'],
        viewport: { width, height: 900 },
      }),
    ),
  );
  for (const context of contexts)
    await context.addInitScript(() => {
      localStorage.setItem('pingo-sound-checked', 'true');
      localStorage.setItem(
        'pingo-audio',
        JSON.stringify({ suppression: 'Off', studio: false, microphone: 'default', speaker: 'default' }),
      );
      navigator.mediaDevices.getDisplayMedia = async () => {
        const canvas = document.createElement('canvas');
        canvas.width = 1280;
        canvas.height = 720;
        const ctx = canvas.getContext('2d')!;
        const draw = () => {
          ctx.fillStyle = '#242427';
          ctx.fillRect(0, 0, 1280, 720);
          ctx.fillStyle = 'white';
          ctx.font = '32px sans-serif';
          ctx.fillText(`Readable shared text ${Date.now()}`, 100, 100);
        };
        draw();
        const stream = canvas.captureStream(15);
        const timer = setInterval(draw, 100);
        stream.getVideoTracks()[0].addEventListener('ended', () => clearInterval(timer));
        return stream;
      };
    });
  const [alice, bob] = await Promise.all(contexts.map((c) => c.newPage()));
  try {
    await signup(alice, 'ShareAlice');
    await signup(bob, 'ShareBob');
    await createSpace(alice);
    await accept(bob, await invite(alice));
    await alice.getByRole('button', { name: 'Start a call', exact: false }).click();
    await bob.getByRole('button', { name: 'Start a call', exact: false }).click();
    await expect(alice.locator('.stage-tile')).toHaveCount(2);
    await expect(bob.getByText('Connecting…', { exact: true })).toHaveCount(0);
    await alice.getByRole('button', { name: 'Share your screen', exact: true }).click();
    const video = bob.getByLabel('Shared screen', { exact: true });
    await expect.poll(() => video.evaluate((node: HTMLVideoElement) => node.videoWidth)).toBeGreaterThan(0);
    await bob.locator('.shared-screen').focus();
    await bob.keyboard.press('Enter');
    await expect(bob.locator('.share-pointer')).toBeVisible();
    await expect(alice.locator('.share-pointer')).toBeVisible();
    await expect(alice.locator('.shared-screen')).not.toHaveAttribute('data-share-mid', '');

    const frame = bob.locator('.share-frame');
    const bounds = (await frame.boundingBox())!;
    expect(bounds.width / bounds.height).toBeCloseTo(16 / 9, 2);
    await bob.mouse.move(bounds.x + bounds.width * 0.25, bounds.y + bounds.height * 0.75);
    const remotePointer = alice.locator('.share-pointer').filter({ hasText: 'ShareBob' });
    await expect.poll(() => remotePointer.evaluate((node) => parseFloat(node.style.left))).toBeCloseTo(25, 0);
    await expect.poll(() => remotePointer.evaluate((node) => parseFloat(node.style.top))).toBeCloseTo(75, 0);

    await bob.getByRole('button', { name: 'Draw on screen', exact: true }).click();
    await bob.mouse.move(bounds.x + bounds.width * 0.2, bounds.y + bounds.height * 0.4);
    await bob.mouse.down();
    await bob.mouse.move(bounds.x + bounds.width * 0.7, bounds.y + bounds.height * 0.65, { steps: 120 });
    await bob.mouse.up();
    await expect(bob.locator('.annotation-stroke').first()).toBeVisible();
    const remoteStroke = alice.locator('.annotation-stroke').first();
    await expect(remoteStroke).toBeVisible();
    expect(await remoteStroke.evaluate((node) => getComputedStyle(node).stroke)).toBe('rgb(255, 204, 51)');

    // The sharer's outgoing mid must also update: their marks need to reach viewers.
    await alice.locator('.shared-screen').focus();
    await alice.keyboard.press('Enter');
    await expect(bob.locator('.share-pointer').filter({ hasText: 'ShareAlice' })).toBeVisible();

    await bob.getByRole('button', { name: 'Zoom in', exact: true }).click();
    await expect.poll(async () => (await frame.boundingBox())!.width).toBeGreaterThan(bounds.width * 1.2);
    expect(await bob.locator('.shared-screen').evaluate((node) => node.scrollWidth > node.clientWidth)).toBe(
      true,
    );
    await bob.getByRole('button', { name: 'Fit shared screen', exact: true }).click();
    await expect.poll(async () => (await frame.boundingBox())!.width).toBeCloseTo(bounds.width, 0);
    await bob.getByRole('button', { name: 'Expand shared screen', exact: true }).click();
    await expect.poll(async () => (await frame.boundingBox())!.width).toBeGreaterThan(bounds.width);
    await bob.keyboard.press('Escape');
    await expect(bob.getByRole('button', { name: 'Expand shared screen', exact: true })).toBeVisible();
    await expect(frame).toBeVisible();

    for (const participant of [alice, bob]) {
      await participant.getByRole('button', { name: 'Zoom in', exact: true }).click();
      await participant.getByRole('button', { name: 'Draw on screen', exact: true }).click();
      await participant.getByRole('button', { name: 'Expand shared screen', exact: true }).click();
      await expect(participant.locator('.stage-tile').first()).toBeHidden();
    }
    await alice.getByRole('button', { name: 'Stop sharing', exact: true }).click();
    for (const participant of [alice, bob]) {
      await expect(participant.locator('.shared-screen')).toHaveCount(0);
      await expect(participant.locator('.stage-tile').first()).toBeVisible();
    }
    await alice.getByRole('button', { name: 'Share your screen', exact: true }).click();
    await expect.poll(() => video.evaluate((node: HTMLVideoElement) => node.videoWidth)).toBeGreaterThan(0);
    for (const participant of [alice, bob]) {
      await expect(
        participant.getByRole('button', { name: 'Expand shared screen', exact: true }),
      ).toHaveAttribute('aria-pressed', 'false');
      await expect(participant.getByRole('button', { name: 'Fit shared screen', exact: true })).toHaveText(
        'Fit',
      );
      await expect(participant.getByRole('button', { name: 'Point on screen', exact: true })).toHaveAttribute(
        'aria-pressed',
        'true',
      );
      await expect(participant.locator('.stage-tile').first()).toBeVisible();
    }
  } finally {
    await Promise.all(contexts.map((c) => c.close()));
  }
});
