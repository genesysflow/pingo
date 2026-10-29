import type { ImageSegmenter } from '@mediapipe/tasks-vision';
import type { BlurDelegate } from './blur-worker-protocol';
import { createSegmenter } from './segmentation';

export function personMask(confidence: Float32Array, pixels: Uint8ClampedArray) {
  for (let index = 0; index < confidence.length; index++) {
    const offset = index * 4;
    pixels[offset] = pixels[offset + 1] = pixels[offset + 2] = 255;
    pixels[offset + 3] = Math.round(Math.max(0, Math.min(1, (confidence[index] - 0.1) / 0.8)) * 255);
  }
}

export class BlurWorkerEngine {
  delegate: BlurDelegate = 'GPU';
  private segmenter?: ImageSegmenter;
  private mask = new OffscreenCanvas(1, 1);

  constructor(private baseUrl: string) {}

  async initialize() {
    try {
      this.segmenter = await createSegmenter(this.baseUrl, 'GPU');
    } catch {
      this.delegate = 'CPU';
      this.segmenter = await createSegmenter(this.baseUrl, 'CPU');
    }
  }

  private infer(image: ImageBitmap, timestamp: number) {
    if (!this.segmenter) throw new Error('Background segmentation is not ready.');
    let produced = false;
    this.segmenter.segmentForVideo(image, timestamp, (result) => {
      const mask = result.confidenceMasks?.[0];
      if (!mask) throw new Error('Background blur did not produce a person mask.');
      if (this.mask.width !== mask.width || this.mask.height !== mask.height) {
        this.mask.width = mask.width;
        this.mask.height = mask.height;
      }
      const context = this.mask.getContext('2d');
      if (!context) throw new Error('Could not create the background mask canvas.');
      const pixels = context.createImageData(mask.width, mask.height);
      personMask(mask.getAsFloat32Array(), pixels.data);
      context.putImageData(pixels, 0, 0);
      produced = true;
    });
    if (!produced) throw new Error('Background blur did not produce a person mask.');
    return this.mask.transferToImageBitmap();
  }

  async segment(image: ImageBitmap, timestamp: number) {
    try {
      try {
        return this.infer(image, timestamp);
      } catch (error) {
        if (this.delegate === 'CPU') throw error;
        this.segmenter?.close();
        this.segmenter = undefined;
        this.delegate = 'CPU';
        this.segmenter = await createSegmenter(this.baseUrl, 'CPU');
        return this.infer(image, timestamp);
      }
    } finally {
      image.close();
    }
  }

  stop() {
    this.segmenter?.close();
    this.segmenter = undefined;
    this.mask.width = this.mask.height = 0;
  }
}
