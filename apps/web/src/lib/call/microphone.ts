import workletUrl from './denoise-worklet.ts?worker&url';
import type { DenoiseMessage, DenoiseOptions } from './denoise-worklet';
import { microphoneConstraints } from './media';
import type { AudioSettings, NoiseEngine } from './controller.svelte';

const wasmUrl = `${import.meta.env.BASE_URL}denoise/pingo-denoise.wasm`;
let compiled: Promise<WebAssembly.Module> | undefined;
/** Compiled once per page; the browser caches the download across calls. */
function denoiseModule() {
  compiled ??= fetch(wasmUrl).then(async (response) => {
    if (!response.ok) throw new Error(`noise suppression model: HTTP ${response.status}`);
    // Not compileStreaming: static hosts don't all send application/wasm.
    return WebAssembly.compile(await response.arrayBuffer());
  });
  compiled.catch(() => (compiled = undefined));
  return compiled;
}

function ready(node: AudioWorkletNode) {
  return new Promise<number>((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('noise suppression did not start')), 10_000);
    node.port.onmessage = ({ data }: MessageEvent<DenoiseMessage>) => {
      clearTimeout(timeout);
      if (data.type === 'ready') resolve(data.delay);
      else reject(new Error(data.message));
      node.port.onmessage = ({ data }: MessageEvent<DenoiseMessage>) => {
        if (data.type === 'error')
          console.warn('Noise suppression stopped; sending the raw microphone.', data.message);
      };
    };
  });
}

type Options = Pick<AudioSettings, 'microphone' | 'suppression' | 'studio'>;

/**
 * The microphone as sent to a call: in Voice mode with suppression on, the raw
 * capture runs through DeepFilterNet3 in an AudioWorklet (the same model and
 * settings as the desktop app), on its own 48 kHz context so model start-up
 * never stalls call playback. Falls back to the browser's suppressor.
 */
export class Microphone {
  readonly stream: MediaStream;
  private constructor(
    private raw: MediaStream,
    readonly track: MediaStreamTrack,
    readonly engine: NoiseEngine,
    private device: string,
    private graph?: { context: AudioContext; node: AudioWorkletNode },
    /** Samples of delay added by the suppressor. */
    readonly delay = 0,
  ) {
    this.stream = new MediaStream([track]);
  }

  static async open(options: Options): Promise<Microphone> {
    const filter = !options.studio && options.suppression !== 'Off';
    if (filter && typeof AudioWorkletNode !== 'undefined' && typeof WebAssembly !== 'undefined') {
      let raw: MediaStream | undefined, context: AudioContext | undefined;
      try {
        const [module, capture] = await Promise.all([
          denoiseModule(),
          navigator.mediaDevices.getUserMedia({
            audio: microphoneConstraints(false, options.microphone, false),
          }),
        ]);
        raw = capture;
        context = new AudioContext({ sampleRate: 48_000, latencyHint: 'interactive' });
        await context.resume();
        await context.audioWorklet.addModule(workletUrl);
        const node = new AudioWorkletNode(context, 'pingo-denoise', {
          numberOfInputs: 1,
          numberOfOutputs: 1,
          outputChannelCount: [1],
          channelCount: 1,
          channelCountMode: 'explicit',
          channelInterpretation: 'speakers',
          processorOptions: { module, strong: options.suppression === 'Strong' } satisfies DenoiseOptions,
        });
        const delay = await ready(node);
        const destination = context.createMediaStreamDestination();
        destination.channelCount = 1;
        context.createMediaStreamSource(raw).connect(node).connect(destination);
        return new Microphone(
          raw,
          destination.stream.getAudioTracks()[0],
          'DeepFilterNet',
          options.microphone,
          { context, node },
          delay,
        );
      } catch (error) {
        raw?.getTracks().forEach((t) => t.stop());
        void context?.close();
        // Permission and device errors would fail the same way below; surface them.
        if (
          error instanceof DOMException &&
          ['NotAllowedError', 'NotFoundError', 'OverconstrainedError'].includes(error.name)
        )
          throw error;
        console.warn('DeepFilterNet is unavailable; using the browser noise suppressor.', error);
      }
    }
    const raw = await navigator.mediaDevices.getUserMedia({
      audio: microphoneConstraints(options.studio, options.microphone, filter),
    });
    return new Microphone(raw, raw.getAudioTracks()[0], filter ? 'Browser' : 'Off', options.microphone);
  }

  /** Applies Gentle/Strong in place, without reopening the microphone. False if `options` needs a new one. */
  retune(options: Options) {
    if (this.engine !== 'DeepFilterNet' || options.studio || options.suppression === 'Off') return false;
    if (options.microphone !== this.device || !this.graph) return false;
    this.graph.node.port.postMessage({ type: 'strength', strong: options.suppression === 'Strong' });
    return true;
  }

  set enabled(enabled: boolean) {
    for (const track of this.raw.getAudioTracks()) track.enabled = enabled;
    this.track.enabled = enabled;
  }

  stop() {
    this.graph?.node.port.postMessage({ type: 'close' });
    this.graph?.node.disconnect();
    this.raw.getTracks().forEach((t) => t.stop());
    this.track.stop();
    void this.graph?.context.close();
    this.graph = undefined;
  }
}
