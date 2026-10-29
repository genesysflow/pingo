import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { BlurWorkerEngine, personMask } from '../src/lib/call/blur-worker-engine';

const factory = vi.hoisted(() => ({ create: vi.fn() }));
vi.mock('../src/lib/call/segmentation', () => ({ createSegmenter: factory.create }));

let segment: ReturnType<typeof vi.fn>;
let close: ReturnType<typeof vi.fn>;
let mask: ImageBitmap;
function image() {
  return { close: vi.fn() } as unknown as ImageBitmap;
}

beforeEach(() => {
  vi.clearAllMocks();
  mask = image();
  segment = vi.fn((_image, _timestamp, callback) =>
    callback({
      confidenceMasks: [{ width: 2, height: 1, getAsFloat32Array: () => new Float32Array([0, 1]) }],
    }),
  );
  close = vi.fn();
  factory.create.mockResolvedValue({ segmentForVideo: segment, close });
  vi.stubGlobal(
    'OffscreenCanvas',
    class {
      width = 1;
      height = 1;
      getContext() {
        return {
          createImageData: (width: number, height: number) => ({
            data: new Uint8ClampedArray(width * height * 4),
          }),
          putImageData: vi.fn(),
        };
      }
      transferToImageBitmap() {
        return mask;
      }
    },
  );
});
afterEach(() => vi.unstubAllGlobals());

test('keeps people opaque, background transparent, and softens uncertain edges', () => {
  const pixels = new Uint8ClampedArray(12);
  personMask(new Float32Array([0, 0.5, 1]), pixels);
  expect(Array.from(pixels)).toEqual([255, 255, 255, 0, 255, 255, 255, 128, 255, 255, 255, 255]);
});

test('prefers GPU inference and closes each transferred input', async () => {
  const engine = new BlurWorkerEngine('https://pingo.test/');
  await engine.initialize();
  const frame = image();
  expect(await engine.segment(frame, 100)).toBe(mask);
  expect(factory.create).toHaveBeenCalledWith('https://pingo.test/', 'GPU');
  expect(engine.delegate).toBe('GPU');
  expect(frame.close).toHaveBeenCalledOnce();
  engine.stop();
  engine.stop();
  expect(close).toHaveBeenCalledOnce();
});

test('falls back to CPU in the worker when GPU initialization is unsupported', async () => {
  factory.create.mockRejectedValueOnce(new Error('GPU unavailable'));
  const engine = new BlurWorkerEngine('https://pingo.test/');
  await engine.initialize();
  expect(factory.create).toHaveBeenNthCalledWith(2, 'https://pingo.test/', 'CPU');
  expect(engine.delegate).toBe('CPU');
  const frame = image();
  expect(await engine.segment(frame, 100)).toBe(mask);
  expect(frame.close).toHaveBeenCalledOnce();
  engine.stop();
});

test('recovers from a lost GPU context by retrying the current frame with CPU inference', async () => {
  const engine = new BlurWorkerEngine('https://pingo.test/');
  await engine.initialize();
  segment.mockImplementationOnce(() => {
    throw new Error('context lost');
  });
  const frame = image();
  expect(await engine.segment(frame, 100)).toBe(mask);
  expect(engine.delegate).toBe('CPU');
  expect(factory.create).toHaveBeenNthCalledWith(2, 'https://pingo.test/', 'CPU');
  expect(close).toHaveBeenCalledOnce();
  expect(frame.close).toHaveBeenCalledOnce();
  engine.stop();
});

test('propagates CPU failures and still releases the input bitmap', async () => {
  factory.create.mockRejectedValueOnce(new Error('GPU unavailable'));
  const engine = new BlurWorkerEngine('https://pingo.test/');
  await engine.initialize();
  segment.mockImplementationOnce(() => {
    throw new Error('inference failed');
  });
  const frame = image();
  await expect(engine.segment(frame, 100)).rejects.toThrow('inference failed');
  expect(frame.close).toHaveBeenCalledOnce();
  engine.stop();
});
