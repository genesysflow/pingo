import type { ImageSegmenter } from '@mediapipe/tasks-vision';

export function supportsBackgroundBlur() {
  if (typeof document === 'undefined' || typeof WebAssembly === 'undefined') return false;
  const canvas = document.createElement('canvas');
  return (
    typeof canvas.captureStream === 'function' &&
    !!canvas.getContext('2d') &&
    'filter' in canvas.getContext('2d')!
  );
}

export function personMask(confidence: Float32Array, pixels: Uint8ClampedArray) {
  for (let index = 0; index < confidence.length; index++) {
    const offset = index * 4;
    pixels[offset] = pixels[offset + 1] = pixels[offset + 2] = 255;
    pixels[offset + 3] = Math.round(Math.max(0, Math.min(1, (confidence[index] - 0.1) / 0.8)) * 255);
  }
}

export class BackgroundBlur {
  readonly stream: MediaStream;
  private frame = 0;
  private stopped = false;
  private lastTime = -1;
  private lastFrame = -Infinity;
  private mask = document.createElement('canvas');
  private foreground = document.createElement('canvas');

  private constructor(
    private video: HTMLVideoElement,
    private canvas: HTMLCanvasElement,
    private segmenter: ImageSegmenter,
    private failed: (error: unknown) => void,
  ) {
    const scale = Math.min(1, 1280 / video.videoWidth, 720 / video.videoHeight);
    canvas.width = this.foreground.width = Math.max(1, Math.round(video.videoWidth * scale));
    canvas.height = this.foreground.height = Math.max(1, Math.round(video.videoHeight * scale));
    this.render(performance.now());
    this.stream = canvas.captureStream(15);
    if (!this.stream.getVideoTracks().length) throw new Error('Could not capture blurred video.');
    this.stream.getVideoTracks()[0].contentHint = 'motion';
    this.frame = requestAnimationFrame(this.tick);
  }

  static async open(raw: MediaStream, failed: (error: unknown) => void) {
    if (!supportsBackgroundBlur()) throw new Error('Background blur is not supported in this browser.');
    const video = document.createElement('video');
    video.muted = true;
    video.playsInline = true;
    video.srcObject = raw;
    let segmenter: ImageSegmenter | undefined;
    let timeout: ReturnType<typeof setTimeout> | undefined;
    let abandoned = false;
    try {
      const setup = (async () => {
        await video.play();
        if (!video.videoWidth || !video.videoHeight) throw new Error('The camera has no video frames.');
        const { createSegmenter } = await import('./segmentation');
        const loaded = await createSegmenter();
        if (abandoned) {
          loaded.close();
          throw new Error('Background blur was cancelled.');
        }
        segmenter = loaded;
      })();
      await Promise.race([
        setup,
        new Promise<never>((_, reject) => {
          timeout = setTimeout(() => reject(new Error('Background blur took too long to start.')), 20_000);
        }),
      ]);
      if (!segmenter) throw new Error('Background blur could not start.');
      return new BackgroundBlur(video, document.createElement('canvas'), segmenter, failed);
    } catch (error) {
      abandoned = true;
      segmenter?.close();
      video.pause();
      video.srcObject = null;
      throw error;
    } finally {
      clearTimeout(timeout);
    }
  }

  private render(timestamp: number) {
    const context = this.canvas.getContext('2d')!;
    const foreground = this.foreground.getContext('2d')!;
    this.segmenter.segmentForVideo(this.video, timestamp, (result) => {
      const mask = result.confidenceMasks?.[0];
      if (!mask) throw new Error('Background blur did not produce a person mask.');
      this.mask.width = mask.width;
      this.mask.height = mask.height;
      const maskContext = this.mask.getContext('2d')!;
      const image = maskContext.createImageData(mask.width, mask.height);
      personMask(mask.getAsFloat32Array(), image.data);
      maskContext.putImageData(image, 0, 0);
    });
    const { width, height } = this.canvas;
    foreground.clearRect(0, 0, width, height);
    foreground.globalCompositeOperation = 'source-over';
    foreground.filter = 'blur(3px)';
    foreground.drawImage(this.mask, 0, 0, width, height);
    foreground.filter = 'none';
    foreground.globalCompositeOperation = 'source-in';
    foreground.drawImage(this.video, 0, 0, width, height);
    foreground.globalCompositeOperation = 'source-over';
    context.filter = 'blur(20px)';
    context.drawImage(this.video, -40, -40, width + 80, height + 80);
    context.filter = 'none';
    context.drawImage(this.foreground, 0, 0);
  }

  private tick = (timestamp: number) => {
    if (this.stopped) return;
    try {
      if (
        this.video.readyState >= 2 &&
        this.video.currentTime !== this.lastTime &&
        timestamp - this.lastFrame >= 1000 / 15
      ) {
        this.render(timestamp);
        this.lastTime = this.video.currentTime;
        this.lastFrame = timestamp;
      }
      this.frame = requestAnimationFrame(this.tick);
    } catch (error) {
      this.stop();
      this.failed(error);
    }
  };

  stop() {
    if (this.stopped) return;
    this.stopped = true;
    cancelAnimationFrame(this.frame);
    this.stream.getTracks().forEach((track) => track.stop());
    this.segmenter.close();
    this.video.pause();
    this.video.srcObject = null;
    this.canvas.width = this.canvas.height = this.foreground.width = this.foreground.height = 0;
  }
}
