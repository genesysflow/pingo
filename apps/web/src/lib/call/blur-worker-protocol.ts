export type BlurDelegate = 'GPU' | 'CPU';

export type BlurWorkerRequest =
  { type: 'init'; baseUrl: string } | { type: 'frame'; image: ImageBitmap; timestamp: number };

export type BlurWorkerResponse =
  | { type: 'loaded' }
  | { type: 'ready'; delegate: BlurDelegate }
  | { type: 'mask'; image: ImageBitmap; timestamp: number; duration: number; delegate: BlurDelegate }
  | { type: 'error'; message: string };
