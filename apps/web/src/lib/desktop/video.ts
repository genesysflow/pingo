import { invoke, Channel } from '@tauri-apps/api/core';
export async function drawNativeFrame(canvas: HTMLCanvasElement, buffer: ArrayBuffer) {
  if (buffer.byteLength < 24) throw new Error('Incomplete native video header');
  const header = new DataView(buffer);
  const id = header.getUint32(0, true), width = header.getUint32(4, true), height = header.getUint32(8, true), format = header.getUint32(12, true);
  if (!width || !height || width > 3840 || height > 2160) throw new Error('Invalid native video dimensions');
  if (canvas.width !== width) canvas.width = width;
  if (canvas.height !== height) canvas.height = height;
  const context = canvas.getContext('2d')!;
  if (format === 0) {
    if (buffer.byteLength !== 24 + width * height * 4) throw new Error('Invalid native frame size');
    context.putImageData(new ImageData(new Uint8ClampedArray(buffer, 24), width, height), 0, 0);
  } else if (format === 1) {
    const bitmap = await createImageBitmap(new Blob([buffer.slice(24)], { type: 'image/jpeg' }));
    try { context.drawImage(bitmap, 0, 0, width, height); } finally { bitmap.close(); }
  } else throw new Error('Unknown native frame format');
  return { id, latencyMs: Date.now() - header.getFloat64(16, true) };
}
export function nativeVideo(sessionId: string, mid: string, canvas: HTMLCanvasElement, error: (error: unknown) => void) {
  let disposed = false, animation = 0;
  const channel = new Channel<ArrayBuffer>();
  channel.onmessage = buffer => {
    animation = requestAnimationFrame(() => {
      if (disposed) return;
      void drawNativeFrame(canvas, buffer).then(({ id }) => {
        if (!disposed) return invoke('video_ack', { key: `${sessionId}:${mid}`, frameId: id });
      }).catch(error);
    });
  };
  void invoke('video_subscribe', { sessionId, mid, channel }).then(() => {
    if (disposed) return invoke('video_unsubscribe', { sessionId, mid });
    return invoke('call_video_quality', { sessionId, mid, maxHeight: 1080 });
  }).catch(error);
  return () => { disposed = true; cancelAnimationFrame(animation); channel.onmessage = () => {}; void invoke('video_unsubscribe', { sessionId, mid }).catch(() => {}); };
}
