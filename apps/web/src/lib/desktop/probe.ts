import { invoke, Channel } from '@tauri-apps/api/core';
import { drawNativeFrame } from './video';
interface ProbeFrame { close(): void }
interface ProbeDecoder {
  configure(config: object): void;
  decode(chunk: unknown): void;
  flush(): Promise<void>;
  close(): void;
}
interface WebCodecs {
  VideoDecoder?: {
    new (callbacks: { output(frame: ProbeFrame): void; error(error: Error): void }): ProbeDecoder;
    isConfigSupported(config: object): Promise<{ supported: boolean }>;
  };
  EncodedVideoChunk?: new (init: { type: string; timestamp: number; data: ArrayBuffer }) => unknown;
}
export async function runVideoProbe(): Promise<boolean> {
  const probe = await invoke<{ video: boolean } | null>('desktop_probe');
  if (!probe?.video) return false;
  const canvas = document.createElement('canvas'); canvas.style.cssText = 'width:100vw;height:90vh;object-fit:contain'; document.body.append(canvas);
  const report: Record<string, unknown> = { userAgent: navigator.userAgent, secureContext: isSecureContext, samples: [] };
  try {
    const { VideoDecoder: decoder, EncodedVideoChunk: Chunk } = window as unknown as WebCodecs;
    for (const [codec, mode] of [['avc1.42E01F', 'encodedH264'], ['vp8', 'encodedVp8']]) {
      const support = decoder ? await decoder.isConfigSupported({ codec, codedWidth: 1920, codedHeight: 1080 }) : { supported: false };
      let encodedBytes = 0, decodedFrames = 0;
      const errors: string[] = [];
      const instance = support.supported && decoder && Chunk ? new decoder({
        output(frame) {
          try { canvas.width = 1920; canvas.height = 1080; canvas.getContext('2d')!.drawImage(frame as unknown as CanvasImageSource, 0, 0); decodedFrames++; }
          finally { frame.close(); }
        },
        error(error) { errors.push(String(error)); },
      }) : undefined;
      instance?.configure({ codec, codedWidth: 1920, codedHeight: 1080 });
      const channel = new Channel<ArrayBuffer>(); channel.onmessage = bytes => {
        encodedBytes += bytes.byteLength;
        if (instance && Chunk) instance.decode(new Chunk({ type: 'key', timestamp: 0, data: bytes }));
      };
      try {
        await invoke('video_spike', { mode, fps: 30, channel });
        await new Promise(r => setTimeout(r, 100));
        await instance?.flush();
      } finally { instance?.close(); }
      report[codec] = { ...support, encodedBytesReceived: encodedBytes, decodedFrames, errors };
    }
    for (const mode of ['h264', 'vp8', 'jpeg_h264', 'jpeg_vp8']) for (const fps of [30, 60]) {
      const latency: number[] = [], errors: string[] = []; let received = 0, callbacks = 0;
      const channel = new Channel<ArrayBuffer>();
      channel.onmessage = bytes => { received++; requestAnimationFrame(() => { callbacks++;
        void drawNativeFrame(canvas, bytes).then(frame => { latency.push(frame.latencyMs); return invoke('video_ack', { key: `spike:${mode}:${fps}`, subscriptionId: `spike:${mode}:${fps}`, frameId: frame.id }); }).catch(e => { errors.push(String(e)); });
      }); };
      const result = await invoke<Record<string, unknown>>('video_spike', { mode, fps, channel });
      latency.sort((a,b) => a-b);
      (report.samples as unknown[]).push({ ...result, received, callbacks, errors, visibility: document.visibilityState, displayed: latency.length, ipcDisplayMedianMs: latency[Math.floor(latency.length*0.5)], ipcDisplayP95Ms: latency[Math.floor(latency.length*0.95)] });
    }
  } catch (e) { report.error = String(e); }
  await invoke('desktop_probe_report', { report });
  return true;
}
