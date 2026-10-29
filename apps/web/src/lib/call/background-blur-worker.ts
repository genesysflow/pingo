import { BlurWorkerEngine } from './blur-worker-engine';
import type { BlurWorkerRequest, BlurWorkerResponse } from './blur-worker-protocol';

const port = self as unknown as Pick<Worker, 'postMessage' | 'onmessage'>;
let engine: BlurWorkerEngine | undefined;

function post(message: BlurWorkerResponse, transfer: Transferable[] = []) {
  port.postMessage(message, transfer);
}

port.onmessage = async ({ data }: MessageEvent<BlurWorkerRequest>) => {
  try {
    if (data.type === 'init') {
      engine = new BlurWorkerEngine(data.baseUrl);
      await engine.initialize();
      post({ type: 'ready', delegate: engine.delegate });
    } else {
      if (!engine) {
        data.image.close();
        throw new Error('Background segmentation is not ready.');
      }
      const started = performance.now();
      const image = await engine.segment(data.image, data.timestamp);
      try {
        post(
          {
            type: 'mask',
            image,
            timestamp: data.timestamp,
            duration: performance.now() - started,
            delegate: engine.delegate,
          },
          [image],
        );
      } catch (error) {
        image.close();
        throw error;
      }
    }
  } catch (error) {
    engine?.stop();
    engine = undefined;
    post({ type: 'error', message: error instanceof Error ? error.message : String(error) });
  }
};

post({ type: 'loaded' });
