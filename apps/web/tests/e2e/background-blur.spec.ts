import { test, expect } from '@playwright/test';

for (const scenario of [
  { delay: 0, disableGpu: false },
  { delay: 100, disableGpu: false },
  { delay: 0, disableGpu: true },
]) {
  test(`real segmentation maintains independent video (mask delay ${scenario.delay} ms, GPU disabled ${scenario.disableGpu})`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.goto('/');
    const result = await page.evaluate(async ({ delay, disableGpu }) => {
      const modulePath = '/src/lib/call/background-blur.ts';
      const { BackgroundBlur } = await import(modulePath);
      const OriginalWorker = window.Worker;
      const bootstrapUrls: string[] = [];
      let workerCount = 0;
      window.Worker = class extends OriginalWorker {
        constructor(url: string | URL, options?: WorkerOptions) {
          super(url, options);
          workerCount++;
        }
        postMessage(message: unknown, transfer: Transferable[] | StructuredSerializeOptions = []) {
          const command = message as { type?: string; url?: string };
          if (disableGpu && command.type === 'load') {
            const injection = `
              const original = OffscreenCanvas.prototype.getContext;
              let fail = true;
              OffscreenCanvas.prototype.getContext = function(type, ...args) {
                if (fail && type === 'webgl2') { fail = false; throw new Error('GPU disabled for test'); }
                return original.call(this, type, ...args);
              };
              await import(${JSON.stringify(command.url)});
            `;
            const url = URL.createObjectURL(new Blob([injection], { type: 'text/javascript' }));
            bootstrapUrls.push(url);
            message = { ...command, url };
          }
          if (Array.isArray(transfer)) super.postMessage(message, transfer);
          else super.postMessage(message, transfer);
        }
        set onmessage(handler: ((this: Worker, event: MessageEvent) => unknown) | null) {
          super.onmessage = handler
            ? function (event) {
                if (delay && event.data.type === 'mask') setTimeout(() => handler.call(this, event), delay);
                else handler.call(this, event);
              }
            : null;
        }
        get onmessage() {
          return super.onmessage;
        }
      };
      const source = document.createElement('canvas');
      source.width = 640;
      source.height = 360;
      const context = source.getContext('2d')!;
      let heartbeat = 0;
      const heartbeatTimer = setInterval(() => heartbeat++, 10);
      const draw = () => {
        for (let column = 0; column < 640; column += 4) {
          context.fillStyle = column % 8 === 0 ? 'black' : 'white';
          context.fillRect(column, 0, 4, 360);
        }
      };
      draw();
      const timer = setInterval(draw, 1000 / 30);
      const raw = source.captureStream(30);
      let failed = '';
      let blur: Awaited<ReturnType<typeof BackgroundBlur.open>> | undefined;
      const video = document.createElement('video');
      video.muted = true;
      try {
        blur = await BackgroundBlur.open(raw, (error: Error) => {
          failed = error.message;
        });
        video.srcObject = blur.stream;
        await video.play();
        let frames = 0;
        let callback = 0;
        const count = () => {
          frames++;
          callback = video.requestVideoFrameCallback(count);
        };
        callback = video.requestVideoFrameCallback(count);
        const started = performance.now();
        const heartbeatStart = heartbeat;
        await new Promise((resolve) => setTimeout(resolve, 2000));
        video.cancelVideoFrameCallback(callback);
        const fps = (frames * 1000) / (performance.now() - started);
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
        const diagnostics = blur.diagnostics;
        const configuredFps = blur.stream.getVideoTracks()[0].getSettings().frameRate;
        blur.stop();
        return {
          deviation,
          mean,
          failed,
          fps,
          configuredFps,
          diagnostics,
          workerCount,
          heartbeats: heartbeat - heartbeatStart,
          processed: blur.stream.getVideoTracks()[0].readyState,
          source: raw.getVideoTracks()[0].readyState,
          dimensions: [video.videoWidth, video.videoHeight],
        };
      } finally {
        blur?.stop();
        raw.getTracks().forEach((track) => track.stop());
        clearInterval(timer);
        clearInterval(heartbeatTimer);
        video.pause();
        video.srcObject = null;
        window.Worker = OriginalWorker;
        bootstrapUrls.forEach((url) => URL.revokeObjectURL(url));
      }
    }, scenario);
    console.log(`Blur performance (${JSON.stringify(scenario)}):`, JSON.stringify(result));
    expect(result.failed).toBe('');
    expect(result.workerCount).toBe(1);
    expect(result.configuredFps).toBe(30);
    expect(result.fps).toBeGreaterThan(20);
    expect(result.heartbeats).toBeGreaterThan(75);
    expect(['GPU', 'CPU']).toContain(result.diagnostics.delegate);
    expect(result.diagnostics.maskLatencyMs).toBeGreaterThan(0);
    if (scenario.delay) expect(result.diagnostics.targetMaskFps).toBeLessThan(15);
    if (scenario.disableGpu) expect(result.diagnostics.delegate).toBe('CPU');
    expect(result.deviation).toBeLessThan(30);
    expect(result.mean).toBeGreaterThan(90);
    expect(result.mean).toBeLessThan(170);
    expect(result.processed).toBe('ended');
    expect(result.source).toBe('live');
    expect(result.dimensions).toEqual([640, 360]);
    expect(errors).toEqual([]);
  });
}
