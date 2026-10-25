import { afterEach, beforeEach, expect, test, vi } from 'vitest';
const bridge = vi.hoisted(() => ({
  invoke: vi.fn(async (_command: string, _args: unknown) => {}),
  channels: [] as { onmessage: (buffer: ArrayBuffer) => void }[],
}));
vi.mock('@tauri-apps/api/core', () => ({
  invoke: bridge.invoke,
  Channel: class {
    onmessage = (_buffer: ArrayBuffer) => {};
    constructor() { bridge.channels.push(this); }
  },
}));
import { drawNativeFrame, nativeVideo } from '../src/lib/desktop/video';
let canvas: HTMLCanvasElement;
let render: FrameRequestCallback | undefined;
async function flush() { for (let i = 0; i < 20; i++) await Promise.resolve(); }
function frame(format = 0) {
  const buffer = new ArrayBuffer(28), view = new DataView(buffer);
  view.setUint32(0, 7, true); view.setUint32(4, 1, true); view.setUint32(8, 1, true);
  view.setUint32(12, format, true); view.setFloat64(16, Date.now(), true);
  return buffer;
}
beforeEach(() => {
  vi.clearAllMocks(); bridge.channels.length = 0; render = undefined;
  canvas = { width: 0, height: 0, getContext: () => ({ putImageData: vi.fn(), drawImage: vi.fn() }), getBoundingClientRect: () => ({ height: 100 }) } as unknown as HTMLCanvasElement;
  vi.stubGlobal('devicePixelRatio', 1);
  vi.stubGlobal('ImageData', class { constructor(public data: Uint8ClampedArray, public width: number, public height: number) {} });
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { render = callback; return 1; });
  vi.stubGlobal('cancelAnimationFrame', vi.fn());
});
afterEach(() => vi.unstubAllGlobals());

test('validates binary dimensions and frame size before allocating pixels', async () => {
  await expect(drawNativeFrame(canvas, new ArrayBuffer(4))).rejects.toThrow('header');
  const bad = frame(); new DataView(bad).setUint32(4, 100000, true);
  await expect(drawNativeFrame(canvas, bad)).rejects.toThrow('dimensions');
  await expect(drawNativeFrame(canvas, frame().slice(0, 24))).rejects.toThrow('size');
  expect(await drawNativeFrame(canvas, frame())).toMatchObject({ id: 7 });
  expect([canvas.width, canvas.height]).toEqual([1, 1]);
});

test('closes decoded bitmaps and releases backpressure even after a display failure', async () => {
  const close = vi.fn(); vi.stubGlobal('createImageBitmap', vi.fn(async () => ({ close })));
  await drawNativeFrame(canvas, frame(1)); expect(close).toHaveBeenCalledOnce();
  const error = vi.fn(), stop = nativeVideo('session', 'mid', canvas, error);
  await flush();
  const subscription = bridge.invoke.mock.calls.find(([name]) => name === 'video_subscribe')![1] as { subscriptionId: string };
  bridge.channels[0].onmessage(frame(99)); render!(0); await flush();
  expect(error).toHaveBeenCalledOnce();
  expect(bridge.invoke).toHaveBeenCalledWith('video_ack', { key: 'session:mid', subscriptionId: subscription.subscriptionId, frameId: 7 });
  stop();
});

test('late subscribe cleanup targets only its own subscription and cancels drawing', async () => {
  let subscribed!: () => void;
  bridge.invoke.mockImplementationOnce(() => new Promise<void>(resolve => { subscribed = resolve; }));
  const error = vi.fn(), stop = nativeVideo('session', 'mid', canvas, error);
  const subscription = bridge.invoke.mock.calls[0][1] as { subscriptionId: string };
  bridge.channels[0].onmessage(frame()); stop();
  subscribed(); await flush(); render!(0); await flush();
  expect(bridge.invoke.mock.calls.filter(([name]) => name === 'video_unsubscribe')).toEqual([
    ['video_unsubscribe', { sessionId: 'session', mid: 'mid', subscriptionId: subscription.subscriptionId }],
    ['video_unsubscribe', { sessionId: 'session', mid: 'mid', subscriptionId: subscription.subscriptionId }],
  ]);
  expect(bridge.invoke.mock.calls.some(([name]) => name === 'video_ack' || name === 'call_video_quality')).toBe(false);
});
