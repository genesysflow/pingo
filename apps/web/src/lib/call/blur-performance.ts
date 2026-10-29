export const BLUR_FPS = 30;
export const MASK_MAX_AGE = 250;

export class BlurPerformance {
  segmentationMs = 0;
  compositeMs = 0;
  private level = 0;
  private slow = 0;
  private fast = 0;

  get maxHeight() {
    return [1080, 720, 540][this.level];
  }

  get segmentationInterval() {
    return Math.max(1000 / BLUR_FPS, Math.min(125, this.segmentationMs * 1.3));
  }

  recordSegmentation(milliseconds: number) {
    this.segmentationMs = this.segmentationMs ? this.segmentationMs * 0.8 + milliseconds * 0.2 : milliseconds;
  }

  recordComposite(milliseconds: number) {
    this.compositeMs = this.compositeMs ? this.compositeMs * 0.9 + milliseconds * 0.1 : milliseconds;
    this.slow = this.compositeMs > 20 ? this.slow + 1 : 0;
    this.fast = this.compositeMs < 8 ? this.fast + 1 : 0;
    if (this.slow >= 15 && this.level < 2) {
      this.level++;
      this.slow = this.fast = 0;
    } else if (this.fast >= 180 && this.level > 0) {
      this.level--;
      this.slow = this.fast = 0;
    }
  }
}

export function fitVideo(width: number, height: number, maxHeight: number) {
  const scale = Math.min(1, maxHeight / height, (maxHeight * 16) / 9 / width);
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) };
}
