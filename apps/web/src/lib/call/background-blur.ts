import workerUrl from './background-blur-worker.ts?worker&url';
import { BLUR_FPS, MASK_MAX_AGE, BlurPerformance, fitVideo } from './blur-performance';
import type { BlurDelegate, BlurWorkerRequest, BlurWorkerResponse } from './blur-worker-protocol';

type Pending = { resolve: () => void; reject: (error: Error) => void };
type PendingFrame = Pending & { timestamp: number; timeout: ReturnType<typeof setTimeout> };

export function supportsBackgroundBlur() {
  if (
    typeof document === 'undefined' ||
    typeof WebAssembly === 'undefined' ||
    typeof Worker === 'undefined' ||
    typeof OffscreenCanvas === 'undefined' ||
    typeof createImageBitmap !== 'function'
  )
    return false;
  const canvas = document.createElement('canvas');
  const context = canvas.getContext('2d');
  return typeof canvas.captureStream === 'function' && !!context && 'filter' in context;
}

export class BackgroundBlur {
  readonly stream: MediaStream;
  private canvas = document.createElement('canvas');
  private foreground = document.createElement('canvas');
  private worker: Worker;
  private baseUrl = new URL(import.meta.env.BASE_URL, document.baseURI).href;
  private frame?: number;
  private videoFrame?: number;
  private stopped = false;
  private active = false;
  private lastTime = -1;
  private lastRender = 0;
  private lastSegmentation = -Infinity;
  private mask?: ImageBitmap;
  private maskTime = -Infinity;
  private delegate: BlurDelegate = 'GPU';
  private performance = new BlurPerformance();
  private ready?: Pending;
  private pending?: PendingFrame;

  get diagnostics() {
    return {
      delegate: this.delegate,
      targetFps: BLUR_FPS,
      width: this.canvas.width,
      height: this.canvas.height,
      maskLatencyMs: this.performance.segmentationMs,
      targetMaskFps: 1000 / this.performance.segmentationInterval,
    };
  }

  private constructor(
    private video: HTMLVideoElement,
    private failed: (error: unknown) => void,
  ) {
    this.resize();
    this.stream = this.canvas.captureStream(BLUR_FPS);
    try {
      if (!this.stream.getVideoTracks().length) throw new Error('Could not capture blurred video.');
      this.stream.getVideoTracks()[0].contentHint = 'motion';
      this.worker = new Worker(new URL('background-blur/worker.js', this.baseUrl));
    } catch (error) {
      this.stream.getTracks().forEach((track) => track.stop());
      throw error;
    }
    this.worker.onmessage = ({ data }: MessageEvent<BlurWorkerResponse>) => this.receive(data);
    this.worker.onmessageerror = () => this.fail(new Error('Background blur worker sent an invalid frame.'));
    this.worker.onerror = (event) => {
      event.preventDefault();
      this.fail(new Error(event.message || 'Background blur worker failed.'));
    };
  }

  static async open(raw: MediaStream, failed: (error: unknown) => void, signal?: AbortSignal) {
    if (!supportsBackgroundBlur()) throw new Error('Background blur is not supported in this browser.');
    if (signal?.aborted) throw new DOMException('Background blur was cancelled.', 'AbortError');
    const video = document.createElement('video');
    video.muted = true;
    video.playsInline = true;
    video.srcObject = raw;
    let blur: BackgroundBlur | undefined;
    let timeout: ReturnType<typeof setTimeout> | undefined;
    let abandoned = false;
    let cancel: (() => void) | undefined;
    try {
      const setup = (async () => {
        await video.play();
        if (abandoned) throw new Error('Background blur was cancelled.');
        if (!video.videoWidth || !video.videoHeight) throw new Error('The camera has no video frames.');
        blur = new BackgroundBlur(video, failed);
        await blur.start();
        return blur;
      })();
      return await Promise.race([
        setup,
        new Promise<never>((_, reject) => {
          timeout = setTimeout(() => reject(new Error('Background blur took too long to start.')), 20_000);
        }),
        new Promise<never>((_, reject) => {
          cancel = () => reject(new DOMException('Background blur was cancelled.', 'AbortError'));
          signal?.addEventListener('abort', cancel, { once: true });
          if (signal?.aborted) cancel();
        }),
      ]);
    } catch (error) {
      abandoned = true;
      blur?.stop();
      video.pause();
      video.srcObject = null;
      throw error;
    } finally {
      clearTimeout(timeout);
      if (cancel) signal?.removeEventListener('abort', cancel);
    }
  }

  private async start() {
    await new Promise<void>((resolve, reject) => {
      this.ready = { resolve, reject };
      this.worker.postMessage({ type: 'load', url: new URL(workerUrl, this.baseUrl).href });
    });
    await this.segment(performance.now());
    this.render(performance.now());
    this.active = true;
    this.lastRender = performance.now();
    this.lastTime = this.video.currentTime;
    this.schedule();
  }

  private receive(message: BlurWorkerResponse) {
    if (this.stopped) {
      if (message.type === 'mask') message.image.close();
      return;
    }
    if (message.type === 'loaded') {
      const request: BlurWorkerRequest = { type: 'init', baseUrl: this.baseUrl };
      this.worker.postMessage(request);
    } else if (message.type === 'ready') {
      this.delegate = message.delegate;
      const ready = this.ready;
      this.ready = undefined;
      ready?.resolve();
    } else if (message.type === 'mask') {
      const pending = this.pending;
      if (!pending || pending.timestamp !== message.timestamp) {
        message.image.close();
        return;
      }
      this.pending = undefined;
      clearTimeout(pending.timeout);
      this.mask?.close();
      this.mask = message.image;
      this.maskTime = message.timestamp;
      this.delegate = message.delegate;
      this.performance.recordSegmentation(Math.max(message.duration, performance.now() - message.timestamp));
      pending.resolve();
    } else {
      this.fail(new Error(message.message));
    }
  }

  private async segment(timestamp: number) {
    if (this.stopped || this.pending) return;
    this.lastSegmentation = timestamp;
    await new Promise<void>((resolve, reject) => {
      this.pending = {
        resolve,
        reject,
        timestamp,
        timeout: setTimeout(() => this.fail(new Error('Background segmentation stopped responding.')), 5000),
      };
      const scale = Math.min(1, 256 / this.video.videoWidth, 256 / this.video.videoHeight);
      void createImageBitmap(this.video, {
        resizeWidth: Math.max(1, Math.round(this.video.videoWidth * scale)),
        resizeHeight: Math.max(1, Math.round(this.video.videoHeight * scale)),
        resizeQuality: 'low',
      })
        .then((image) => {
          if (this.stopped) {
            image.close();
            return;
          }
          try {
            const request: BlurWorkerRequest = { type: 'frame', image, timestamp };
            this.worker.postMessage(request, [image]);
          } catch (error) {
            image.close();
            this.fail(error instanceof Error ? error : new Error(String(error)));
          }
        })
        .catch((error) => this.fail(error instanceof Error ? error : new Error(String(error))));
    });
  }

  private resize() {
    const size = fitVideo(this.video.videoWidth, this.video.videoHeight, this.performance.maxHeight);
    if (this.canvas.width === size.width && this.canvas.height === size.height) return;
    this.canvas.width = this.foreground.width = size.width;
    this.canvas.height = this.foreground.height = size.height;
  }

  private render(timestamp: number) {
    const started = performance.now();
    this.resize();
    const context = this.canvas.getContext('2d')!;
    const { width, height } = this.canvas;
    context.filter = `blur(${Math.max(12, height / 36)}px)`;
    context.drawImage(this.video, -60, -60, width + 120, height + 120);
    context.filter = 'none';
    if (this.mask && timestamp - this.maskTime <= MASK_MAX_AGE) {
      const foreground = this.foreground.getContext('2d')!;
      foreground.clearRect(0, 0, width, height);
      foreground.globalCompositeOperation = 'source-over';
      foreground.filter = 'blur(3px)';
      foreground.drawImage(this.mask, 0, 0, width, height);
      foreground.filter = 'none';
      foreground.globalCompositeOperation = 'source-in';
      foreground.drawImage(this.video, 0, 0, width, height);
      foreground.globalCompositeOperation = 'source-over';
      context.drawImage(this.foreground, 0, 0);
    }
    this.performance.recordComposite(performance.now() - started);
  }

  private schedule() {
    if (this.video.requestVideoFrameCallback)
      this.videoFrame = this.video.requestVideoFrameCallback(this.tick);
    else this.frame = requestAnimationFrame(this.tick);
  }

  private tick = (timestamp: number) => {
    if (this.stopped) return;
    try {
      if (this.video.readyState >= 2 && this.video.currentTime !== this.lastTime) {
        if (!this.pending && timestamp - this.lastSegmentation >= this.performance.segmentationInterval)
          void this.segment(timestamp).catch((error) => this.fail(error));
        const interval = 1000 / BLUR_FPS;
        const elapsed = timestamp - this.lastRender;
        if (elapsed >= interval - 2) {
          this.render(timestamp);
          this.lastRender += Math.max(1, Math.floor((elapsed + 2) / interval)) * interval;
          this.lastTime = this.video.currentTime;
        }
      }
      this.schedule();
    } catch (error) {
      this.fail(error instanceof Error ? error : new Error(String(error)));
    }
  };

  private fail(error: Error) {
    if (this.stopped) return;
    const notify = this.active;
    this.stop(error);
    if (notify) this.failed(error);
  }

  stop(error = new Error('Background blur stopped.')) {
    if (this.stopped) return;
    this.stopped = true;
    if (this.frame !== undefined) cancelAnimationFrame(this.frame);
    if (this.videoFrame !== undefined) this.video.cancelVideoFrameCallback(this.videoFrame);
    this.worker.terminate();
    this.worker.onmessage = this.worker.onerror = this.worker.onmessageerror = null;
    this.ready?.reject(error);
    this.ready = undefined;
    if (this.pending) {
      clearTimeout(this.pending.timeout);
      this.pending.reject(error);
      this.pending = undefined;
    }
    this.mask?.close();
    this.mask = undefined;
    this.stream.getTracks().forEach((track) => track.stop());
    this.video.pause();
    this.video.srcObject = null;
    this.canvas.width = this.canvas.height = this.foreground.width = this.foreground.height = 0;
  }
}
