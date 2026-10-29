import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import { BackgroundBlur, personMask, supportsBackgroundBlur } from '../src/lib/call/background-blur';

const model = vi.hoisted(() => ({ create: vi.fn(), segment: vi.fn(), close: vi.fn() }));
vi.mock('../src/lib/call/segmentation', () => ({ createSegmenter: model.create }));

let video: {
  play: ReturnType<typeof vi.fn>;
  pause: ReturnType<typeof vi.fn>;
  srcObject: unknown;
  videoWidth: number;
  videoHeight: number;
  readyState: number;
  currentTime: number;
};
let outputTrack: { stop: ReturnType<typeof vi.fn>; contentHint: string };
let rawTrack: { stop: ReturnType<typeof vi.fn> };
let raw: MediaStream;
let context: CanvasRenderingContext2D;
let tick: FrameRequestCallback;

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
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
  const output = { getTracks: () => [outputTrack], getVideoTracks: () => [outputTrack] };
  context = {
    filter: 'none',
    drawImage: vi.fn(),
    clearRect: vi.fn(),
    putImageData: vi.fn(),
    createImageData: (width: number, height: number) => ({ data: new Uint8ClampedArray(width * height * 4) }),
  } as unknown as CanvasRenderingContext2D;
  vi.stubGlobal('document', {
    createElement: (tag: string) =>
      tag === 'video'
        ? video
        : {
            width: 0,
            height: 0,
            getContext: () => context,
            captureStream: () => output,
          },
  });
  vi.stubGlobal(
    'requestAnimationFrame',
    vi.fn((callback: FrameRequestCallback) => {
      tick = callback;
      return 1;
    }),
  );
  vi.stubGlobal('cancelAnimationFrame', vi.fn());
  model.segment.mockImplementation((_video, _time, callback) =>
    callback({
      confidenceMasks: [{ width: 2, height: 1, getAsFloat32Array: () => new Float32Array([0, 1]) }],
    }),
  );
  model.create.mockResolvedValue({ segmentForVideo: model.segment, close: model.close });
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('background blur', () => {
  test('keeps people opaque and the background transparent in the sharp layer', () => {
    const pixels = new Uint8ClampedArray(12);
    personMask(new Float32Array([0, 0.5, 1]), pixels);
    expect(Array.from(pixels)).toEqual([255, 255, 255, 0, 255, 255, 255, 128, 255, 255, 255, 255]);
  });
  test('hides support when canvas filtering or stream capture is missing', () => {
    expect(supportsBackgroundBlur()).toBe(true);
    vi.stubGlobal('document', { createElement: () => ({ getContext: () => ({}) }) });
    expect(supportsBackgroundBlur()).toBe(false);
  });
  test('sends a processed stream and releases only its own resources', async () => {
    const blur = await BackgroundBlur.open(raw, vi.fn());
    expect(blur.stream).not.toBe(raw);
    expect(model.segment).toHaveBeenCalledTimes(1);
    expect(context.drawImage).toHaveBeenCalled();
    expect(outputTrack.contentHint).toBe('motion');
    blur.stop();
    blur.stop();
    expect(outputTrack.stop).toHaveBeenCalledTimes(1);
    expect(rawTrack.stop).not.toHaveBeenCalled();
    expect(model.close).toHaveBeenCalledTimes(1);
    expect(video.srcObject).toBeNull();
    expect(cancelAnimationFrame).toHaveBeenCalledWith(1);
  });
  test('does not reprocess duplicate frames', async () => {
    const blur = await BackgroundBlur.open(raw, vi.fn());
    video.currentTime = 1;
    tick(1000);
    tick(1100);
    expect(model.segment).toHaveBeenCalledTimes(2);
    blur.stop();
  });
  test('stops processed output on inference failure instead of sending raw video', async () => {
    const failed = vi.fn();
    const blur = await BackgroundBlur.open(raw, failed);
    model.segment.mockImplementationOnce(() => {
      throw new Error('inference failed');
    });
    video.currentTime = 1;
    tick(1000);
    expect(failed).toHaveBeenCalledWith(expect.objectContaining({ message: 'inference failed' }));
    expect(outputTrack.stop).toHaveBeenCalledOnce();
    expect(model.close).toHaveBeenCalledOnce();
    expect(rawTrack.stop).not.toHaveBeenCalled();
    blur.stop();
  });
  test('cleans up failed initialization without producing a stream', async () => {
    model.segment.mockImplementationOnce(() => {
      throw new Error('no mask');
    });
    await expect(BackgroundBlur.open(raw, vi.fn())).rejects.toThrow('no mask');
    expect(model.close).toHaveBeenCalledOnce();
    expect(video.srcObject).toBeNull();
    expect(requestAnimationFrame).not.toHaveBeenCalled();
  });
  test('closes a model that finishes loading after startup timed out', async () => {
    let loaded!: (value: unknown) => void;
    model.create.mockReturnValueOnce(
      new Promise((resolve) => {
        loaded = resolve;
      }),
    );
    const result = expect(BackgroundBlur.open(raw, vi.fn())).rejects.toThrow('too long');
    await vi.advanceTimersByTimeAsync(20_000);
    await result;
    loaded({ close: model.close });
    await Promise.resolve();
    expect(model.close).toHaveBeenCalledOnce();
    expect(video.srcObject).toBeNull();
  });
});
