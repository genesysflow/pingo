import { test, expect } from '@playwright/test';

test('the self-hosted segmentation model blurs transmitted pixels and cleans up', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/');
  const result = await page.evaluate(async () => {
    const modulePath = '/src/lib/call/background-blur.ts';
    const { BackgroundBlur } = await import(modulePath);
    const source = document.createElement('canvas');
    source.width = 640;
    source.height = 360;
    const context = source.getContext('2d')!;
    const draw = () => {
      for (let column = 0; column < 640; column += 4) {
        context.fillStyle = column % 8 === 0 ? 'black' : 'white';
        context.fillRect(column, 0, 4, 360);
      }
    };
    draw();
    const timer = setInterval(draw, 50);
    const raw = source.captureStream(15);
    let failed = '';
    const blur = await BackgroundBlur.open(raw, (error: Error) => {
      failed = error.message;
    });
    const video = document.createElement('video');
    video.muted = true;
    video.srcObject = blur.stream;
    try {
      await video.play();
      await new Promise<void>((resolve) => video.requestVideoFrameCallback(() => resolve()));
      const received = document.createElement('canvas');
      received.width = 640;
      received.height = 360;
      const output = received.getContext('2d')!;
      output.drawImage(video, 0, 0);
      const pixels = output.getImageData(80, 80, 80, 1).data;
      const values = Array.from({ length: 80 }, (_, index) => pixels[index * 4]);
      const mean = values.reduce((sum, value) => sum + value, 0) / values.length;
      const deviation = Math.sqrt(
        values.reduce((sum, value) => sum + (value - mean) ** 2, 0) / values.length,
      );
      blur.stop();
      return {
        deviation,
        mean,
        failed,
        processed: blur.stream.getVideoTracks()[0].readyState,
        source: raw.getVideoTracks()[0].readyState,
        dimensions: [video.videoWidth, video.videoHeight],
      };
    } finally {
      blur.stop();
      raw.getTracks().forEach((track) => track.stop());
      clearInterval(timer);
      video.pause();
      video.srcObject = null;
    }
  });
  expect(result.failed).toBe('');
  expect(result.deviation).toBeLessThan(30);
  expect(result.mean).toBeGreaterThan(90);
  expect(result.mean).toBeLessThan(170);
  expect(result.processed).toBe('ended');
  expect(result.source).toBe('live');
  expect(result.dimensions).toEqual([640, 360]);
  expect(errors).toEqual([]);
});
