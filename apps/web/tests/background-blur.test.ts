import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import { BackgroundBlur, supportsBackgroundBlur } from '../src/lib/call/background-blur';
import type { BlurWorkerResponse } from '../src/lib/call/blur-worker-protocol';

vi.mock('../src/lib/call/background-blur-worker.ts?worker&url', () => ({ default: '/worker-module.js' }));

type FakeVideo = {
  play: ReturnType<typeof vi.fn>;
  pause: ReturnType<typeof vi.fn>;
  srcObject: unknown;
  videoWidth: number;
  videoHeight: number;
  readyState: number;
  currentTime: number;
  requestVideoFrameCallback?: ReturnType<typeof vi.fn>;
  cancelVideoFrameCallback?: ReturnType<typeof vi.fn>;
};

function bitmap() {
  return { width: 256, height: 144, close: vi.fn() } as unknown as ImageBitmap;
}

class FakeWorker {
  onmessage: ((event: MessageEvent<BlurWorkerResponse>) => void) | null = null;
  onerror: ((event: ErrorEvent) => void) | null = null;
  onmessageerror: (() => void) | null = null;
  terminate = vi.fn();
  postMessage = vi.fn((data) => {
    if (data.type === 'load' && initialize) queueMicrotask(() => this.emit({ type: 'loaded' }));
    if (data.type === 'init' && initialize)
      queueMicrotask(() => this.emit({ type: 'ready', delegate: 'GPU' }));
    if (data.type === 'frame' && replyFrames)
      queueMicrotask(() => {
        data.image.close();
        const image = bitmap();
        masks.push(image);
        this.emit({ type: 'mask', image, timestamp: data.timestamp, duration: 5, delegate: 'GPU' });
      });
  });
  constructor() {
    workers.push(this);
  }
  emit(data: BlurWorkerResponse) {
    this.onmessage?.({ data } as MessageEvent<BlurWorkerResponse>);
  }
}

let video: FakeVideo;
let workers: FakeWorker[];
let masks: ImageBitmap[];
let initialize: boolean;
let replyFrames: boolean;
let outputTrack: { stop: ReturnType<typeof vi.fn>; contentHint: string };
let rawTrack: { stop: ReturnType<typeof vi.fn> };
let raw: MediaStream;
let context: CanvasRenderingContext2D;
let capture: ReturnType<typeof vi.fn>;
let tick: FrameRequestCallback;

async function flush() {
  for (let turn = 0; turn < 12; turn++) await Promise.resolve();
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  workers = [];
  masks = [];
  initialize = replyFrames = true;
  video = {
    play: vi.fn().mockResolvedValue(undefined),
    pause: vi.fn(),
    srcObject: null,
    videoWidth: 1920,
    videoHeight: 1080,
    readyState: 4,
    currentTime: 0,
  };
  rawTrack = { stop: vi.fn() };
  outputTrack = { stop: vi.fn(), contentHint: '' };
  raw = { getTracks: () => [rawTrack] } as unknown as MediaStream;
  capture = vi.fn(() => ({ getTracks: () => [outputTrack], getVideoTracks: () => [outputTrack] }));
  context = { filter: 'none', drawImage: vi.fn(), clearRect: vi.fn() } as unknown as CanvasRenderingContext2D;
  vi.stubGlobal('document', {
    baseURI: 'http://pingo.test/app/',
    createElement: (tag: string) =>
      tag === 'video'
        ? video
        : {
            width: 0,
            height: 0,
            getContext: () => context,
            captureStream: capture,
          },
  });
  vi.stubGlobal('Worker', FakeWorker);
  vi.stubGlobal('OffscreenCanvas', class {});
  vi.stubGlobal(
    'createImageBitmap',
    vi.fn(async () => bitmap()),
  );
  vi.stubGlobal(
    'requestAnimationFrame',
    vi.fn((callback: FrameRequestCallback) => {
      tick = callback;
      return 1;
    }),
  );
  vi.stubGlobal('cancelAnimationFrame', vi.fn());
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('background blur', () => {
  test('requires off-thread support and canvas filtering', () => {
    expect(supportsBackgroundBlur()).toBe(true);
    vi.stubGlobal('Worker', undefined);
    expect(supportsBackgroundBlur()).toBe(false);
  });
  test('publishes 1080p30 only after a worker-generated mask, then frees owned resources', async () => {
    const blur = await BackgroundBlur.open(raw, vi.fn());
    expect(blur.stream).not.toBe(raw);
    expect(capture).toHaveBeenCalledWith(30);
    expect(blur.diagnostics).toMatchObject({ width: 1920, height: 1080, targetFps: 30, delegate: 'GPU' });
    expect(createImageBitmap).toHaveBeenCalledWith(video, {
      resizeWidth: 256,
      resizeHeight: 144,
      resizeQuality: 'low',
    });
    expect(context.drawImage).toHaveBeenCalledWith(masks[0], 0, 0, 1920, 1080);
    blur.stop();
    blur.stop();
    expect(outputTrack.stop).toHaveBeenCalledOnce();
    expect(rawTrack.stop).not.toHaveBeenCalled();
    expect(workers[0].terminate).toHaveBeenCalledOnce();
    expect(masks[0].close).toHaveBeenCalledOnce();
    expect(video.srcObject).toBeNull();
    expect(cancelAnimationFrame).toHaveBeenCalledWith(1);
  });
  test('renders at 30 FPS while a single segmentation request is still in flight', async () => {
    const blur = await BackgroundBlur.open(raw, vi.fn());
    replyFrames = false;
    vi.mocked(context.drawImage).mockClear();
    video.currentTime = 1;
    tick(34);
    await flush();
    video.currentTime = 2;
    tick(67);
    video.currentTime = 3;
    tick(100);
    expect(createImageBitmap).toHaveBeenCalledTimes(2);
    expect(context.drawImage).toHaveBeenCalledTimes(12);
    blur.stop();
    await flush();
  });
  test('does not process or render duplicate camera frames', async () => {
    const blur = await BackgroundBlur.open(raw, vi.fn());
    vi.mocked(context.drawImage).mockClear();
    tick(100);
    tick(200);
    expect(createImageBitmap).toHaveBeenCalledTimes(1);
    expect(context.drawImage).not.toHaveBeenCalled();
    blur.stop();
  });
  test('fully blurs fresh video rather than applying an old person mask', async () => {
    const blur = await BackgroundBlur.open(raw, vi.fn());
    replyFrames = false;
    vi.mocked(context.drawImage).mockClear();
    video.currentTime = 1;
    tick(300);
    expect(context.drawImage).toHaveBeenCalledTimes(1);
    expect(context.drawImage).toHaveBeenCalledWith(video, -60, -60, 2040, 1200);
    blur.stop();
    await flush();
  });
  test('stops processed output when the worker fails, without falling back to raw video', async () => {
    const failed = vi.fn();
    const blur = await BackgroundBlur.open(raw, failed);
    workers[0].emit({ type: 'error', message: 'inference failed' });
    expect(failed).toHaveBeenCalledWith(expect.objectContaining({ message: 'inference failed' }));
    expect(outputTrack.stop).toHaveBeenCalledOnce();
    expect(workers[0].terminate).toHaveBeenCalledOnce();
    expect(rawTrack.stop).not.toHaveBeenCalled();
    blur.stop();
  });
  test('terminates a worker whose initialization times out', async () => {
    initialize = false;
    const result = expect(BackgroundBlur.open(raw, vi.fn())).rejects.toThrow('too long');
    await vi.advanceTimersByTimeAsync(20_000);
    await result;
    expect(workers[0].terminate).toHaveBeenCalledOnce();
    expect(outputTrack.stop).toHaveBeenCalledOnce();
    expect(video.srcObject).toBeNull();
  });
  test('immediately terminates pending startup when its call is cancelled', async () => {
    initialize = false;
    const controller = new AbortController();
    const result = expect(BackgroundBlur.open(raw, vi.fn(), controller.signal)).rejects.toMatchObject({
      name: 'AbortError',
    });
    await flush();
    controller.abort();
    await result;
    expect(workers[0].terminate).toHaveBeenCalledOnce();
    expect(outputTrack.stop).toHaveBeenCalledOnce();
    expect(video.srcObject).toBeNull();
  });
  test('bounds a hung segmentation request and fails closed', async () => {
    const failed = vi.fn();
    const blur = await BackgroundBlur.open(raw, failed);
    replyFrames = false;
    video.currentTime = 1;
    tick(34);
    await flush();
    await vi.advanceTimersByTimeAsync(5000);
    expect(failed).toHaveBeenCalledWith(
      expect.objectContaining({ message: 'Background segmentation stopped responding.' }),
    );
    expect(outputTrack.stop).toHaveBeenCalledOnce();
    blur.stop();
  });
  test('closes a bitmap created after shutdown without posting another worker request', async () => {
    const blur = await BackgroundBlur.open(raw, vi.fn());
    let created!: (image: ImageBitmap) => void;
    vi.mocked(createImageBitmap).mockReturnValueOnce(
      new Promise((resolve) => {
        created = resolve;
      }),
    );
    video.currentTime = 1;
    tick(34);
    blur.stop();
    const late = bitmap();
    created(late);
    await flush();
    expect(late.close).toHaveBeenCalledOnce();
    expect(workers[0].postMessage.mock.calls.filter(([data]) => data.type === 'frame')).toHaveLength(1);
  });
  test('uses decoded video callbacks when available and cancels them on shutdown', async () => {
    video.requestVideoFrameCallback = vi.fn((callback) => {
      tick = callback;
      return 4;
    });
    video.cancelVideoFrameCallback = vi.fn();
    const blur = await BackgroundBlur.open(raw, vi.fn());
    expect(video.requestVideoFrameCallback).toHaveBeenCalledOnce();
    expect(requestAnimationFrame).not.toHaveBeenCalled();
    blur.stop();
    expect(video.cancelVideoFrameCallback).toHaveBeenCalledWith(4);
  });
});
