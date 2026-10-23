import { invoke, Channel } from '@tauri-apps/api/core';
import { drawNativeFrame } from './video';
export async function runVideoProbe(): Promise<boolean> {
  const probe = await invoke<{ video: boolean } | null>('desktop_probe');
  if (!probe?.video) return false;
  const canvas = document.createElement('canvas'); canvas.style.cssText = 'width:100vw;height:90vh;object-fit:contain'; document.body.append(canvas);
  const report: Record<string, unknown> = { userAgent: navigator.userAgent, secureContext: isSecureContext, samples: [] };
  try {
    const decoder = (window as unknown as { VideoDecoder?: { isConfigSupported(config: object): Promise<{ supported: boolean }> } }).VideoDecoder;
    for (const [codec, mode] of [['avc1.42E01F', 'encodedH264'], ['vp8', 'encodedVp8']]) {
      const support = decoder ? await decoder.isConfigSupported({ codec, codedWidth: 1920, codedHeight: 1080 }) : { supported: false };
      let encodedBytes = 0;
      const channel = new Channel<ArrayBuffer>(); channel.onmessage = bytes => { encodedBytes += bytes.byteLength; };
      await invoke('video_spike', { mode, fps: 30, channel });
      await new Promise(r => setTimeout(r, 100));
      report[codec] = { ...support, encodedBytesReceived: encodedBytes };
    }
    for (const mode of ['h264', 'vp8', 'jpeg_h264', 'jpeg_vp8']) for (const fps of [30, 60]) {
      const latency: number[] = [], errors: string[] = []; let received = 0, callbacks = 0;
      const channel = new Channel<ArrayBuffer>();
      channel.onmessage = bytes => { received++; requestAnimationFrame(() => { callbacks++;
        void drawNativeFrame(canvas, bytes).then(frame => { latency.push(frame.latencyMs); return invoke('video_ack', { key: `spike:${mode}:${fps}`, frameId: frame.id }); }).catch(e => { errors.push(String(e)); });
      }); };
      const result = await invoke<Record<string, unknown>>('video_spike', { mode, fps, channel });
      latency.sort((a,b) => a-b);
      (report.samples as unknown[]).push({ ...result, received, callbacks, errors, visibility: document.visibilityState, displayed: latency.length, ipcDisplayMedianMs: latency[Math.floor(latency.length*0.5)], ipcDisplayP95Ms: latency[Math.floor(latency.length*0.95)] });
    }
  } catch (e) { report.error = String(e); }
  await invoke('desktop_probe_report', { report });
  return true;
}
