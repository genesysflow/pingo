import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { BrowserCallController } from '../src/lib/call/browser.svelte';

const processor = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock('../src/lib/call/background-blur', () => ({
  supportsBackgroundBlur: () => true,
  BackgroundBlur: { open: processor.open },
}));

let call: BrowserCallController;
function stream() {
  const track = { stop: vi.fn(), contentHint: '', readyState: 'live' };
  return {
    track,
    stream: { getTracks: () => [track], getVideoTracks: () => [track] } as unknown as MediaStream,
  };
}
let source: ReturnType<typeof stream>;
let output: ReturnType<typeof stream>;
let stop: ReturnType<typeof vi.fn>;
let sender: { replaceTrack: ReturnType<typeof vi.fn> };
let publish: ReturnType<typeof vi.fn>;
async function flush() {
  for (let turn = 0; turn < 12; turn++) await Promise.resolve();
  await Reflect.get(call, 'localQueue');
}
beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  vi.stubGlobal('localStorage', { getItem: () => null, setItem: vi.fn() });
  source = stream();
  output = stream();
  stop = vi.fn(() => output.track.stop());
  processor.open.mockResolvedValue({ stream: output.stream, stop });
  sender = { replaceTrack: vi.fn().mockResolvedValue(undefined) };
  publish = vi.fn().mockResolvedValue(undefined);
  call = new BrowserCallController(vi.fn(), vi.fn(), () => 'you');
  Object.assign(call, {
    room: 'room',
    camera: true,
    localCamera: source.stream,
    cameraSource: source.stream,
    webcam: source.stream,
    pc: { close: vi.fn() },
    published: new Map([[{ sender }, 'camera']]),
    offer: vi.fn().mockResolvedValue(undefined),
    addVideo: publish,
  });
});
afterEach(() => {
  call.destroy();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

test('live toggle replaces the transmitted and preview track, then restores the same camera', async () => {
  call.setBackgroundBlur(true);
  expect(call.cameraBusy).toBe(true);
  await flush();
  expect(sender.replaceTrack).toHaveBeenCalledWith(output.track);
  expect(call.localCamera).toBe(output.stream);
  expect(call.backgroundBlur).toBe(true);
  expect(source.track.stop).not.toHaveBeenCalled();
  call.setBackgroundBlur(false);
  await flush();
  expect(sender.replaceTrack).toHaveBeenLastCalledWith(source.track);
  expect(call.localCamera).toBe(source.stream);
  expect(call.backgroundBlur).toBe(false);
  expect(stop).toHaveBeenCalledOnce();
  expect(call.cameraBusy).toBe(false);
  expect(localStorage.setItem).toHaveBeenLastCalledWith('pingo-background-blur', 'false');
});

test('a failed live setup keeps the current camera and preference unchanged', async () => {
  processor.open.mockRejectedValueOnce(new Error('model unavailable'));
  call.setBackgroundBlur(true);
  await flush();
  expect(sender.replaceTrack).not.toHaveBeenCalled();
  expect(call.backgroundBlur).toBe(false);
  expect(call.localCamera).toBe(source.stream);
  expect(call.cameraBusy).toBe(false);
  expect(call.error).toBe('model unavailable');
});

test('a failed sender replacement discards the processor without changing preview', async () => {
  sender.replaceTrack.mockRejectedValueOnce(new Error('replacement failed'));
  call.setBackgroundBlur(true);
  await flush();
  expect(stop).toHaveBeenCalledOnce();
  expect(source.track.stop).not.toHaveBeenCalled();
  expect(call.localCamera).toBe(source.stream);
  expect(call.backgroundBlur).toBe(false);
});

test('leaving during setup closes late resources without publishing them', async () => {
  let loaded!: (value: unknown) => void;
  processor.open.mockReturnValueOnce(
    new Promise((resolve) => {
      loaded = resolve;
    }),
  );
  call.setBackgroundBlur(true);
  await Promise.resolve();
  call.leave();
  expect((processor.open.mock.calls[0][2] as AbortSignal).aborted).toBe(true);
  loaded({ stream: output.stream, stop });
  await flush();
  expect(stop).toHaveBeenCalledOnce();
  expect(source.track.stop).toHaveBeenCalledOnce();
  expect(sender.replaceTrack).not.toHaveBeenCalled();
  expect(call.localCamera).toBeNull();
  expect(call.cameraBusy).toBe(false);
});

test('blur selected before camera startup is applied before the first publication', async () => {
  call.camera = false;
  call.backgroundBlur = true;
  vi.stubGlobal('navigator', { mediaDevices: { getUserMedia: vi.fn().mockResolvedValue(source.stream) } });
  call.toggleCamera();
  await flush();
  expect(publish).toHaveBeenCalledWith(output.stream, 'camera');
  expect(call.localCamera).toBe(output.stream);
  expect(call.cameraBusy).toBe(false);
});

test('startup failure never publishes the unblurred camera', async () => {
  call.camera = false;
  call.backgroundBlur = true;
  processor.open.mockRejectedValueOnce(new Error('model failed'));
  vi.stubGlobal('navigator', { mediaDevices: { getUserMedia: vi.fn().mockResolvedValue(source.stream) } });
  call.toggleCamera();
  await flush();
  expect(publish).not.toHaveBeenCalled();
  expect(source.track.stop).toHaveBeenCalled();
  expect(call.camera).toBe(false);
  expect(call.localCamera).toBeNull();
  expect(call.error).toBe('model failed');
});

test('runtime failures still stop the camera after a reconnect changes the call generation', async () => {
  call.setBackgroundBlur(true);
  await flush();
  Reflect.set(call, 'generation', 1);
  const failed = processor.open.mock.calls[0][1] as (error: Error) => void;
  failed(new Error('inference failed'));
  await flush();
  expect(call.camera).toBe(false);
  expect(call.localCamera).toBeNull();
  expect(source.track.stop).toHaveBeenCalled();
  expect(stop).toHaveBeenCalledOnce();
  expect(sender.replaceTrack).toHaveBeenLastCalledWith(null);
  expect(call.error).toBe('Background blur stopped: inference failed');
  expect(call.backgroundBlur).toBe(true);
});

test('a replacement completing after leave does not negotiate into a new call', async () => {
  let replaced!: () => void;
  sender.replaceTrack.mockReturnValueOnce(
    new Promise<void>((resolve) => {
      replaced = resolve;
    }),
  );
  call.setBackgroundBlur(true);
  for (let turn = 0; turn < 5; turn++) await Promise.resolve();
  call.leave();
  const offer = vi.fn().mockResolvedValue(undefined);
  Reflect.set(call, 'offer', offer);
  replaced();
  await flush();
  expect(offer).not.toHaveBeenCalled();
  expect(stop).toHaveBeenCalledOnce();
});
