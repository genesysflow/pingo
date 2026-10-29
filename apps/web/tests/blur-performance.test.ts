import { expect, test } from 'vitest';
import { BlurPerformance, fitVideo } from '../src/lib/call/blur-performance';

test('targets 1080p and 30 mask updates per second until measurements justify reducing work', () => {
  const performance = new BlurPerformance();
  expect(performance.maxHeight).toBe(1080);
  expect(performance.segmentationInterval).toBeCloseTo(1000 / 30);
  performance.recordSegmentation(10);
  expect(performance.segmentationInterval).toBeCloseTo(1000 / 30);
  for (let frame = 0; frame < 30; frame++) performance.recordSegmentation(100);
  expect(performance.segmentationInterval).toBe(125);
  for (let frame = 0; frame < 30; frame++) performance.recordSegmentation(5);
  expect(performance.segmentationInterval).toBeCloseTo(1000 / 30);
});

test('reduces rendering resolution under sustained load and recovers slowly without oscillating', () => {
  const performance = new BlurPerformance();
  for (let frame = 0; frame < 14; frame++) performance.recordComposite(25);
  expect(performance.maxHeight).toBe(1080);
  performance.recordComposite(25);
  expect(performance.maxHeight).toBe(720);
  for (let frame = 0; frame < 15; frame++) performance.recordComposite(25);
  expect(performance.maxHeight).toBe(540);
  for (let frame = 0; frame < 100; frame++) performance.recordComposite(2);
  expect(performance.maxHeight).toBe(540);
  for (let frame = 0; frame < 300; frame++) performance.recordComposite(2);
  expect(performance.maxHeight).toBe(1080);
});

test('preserves source aspect ratio and never upscales', () => {
  expect(fitVideo(3840, 2160, 1080)).toEqual({ width: 1920, height: 1080 });
  expect(fitVideo(1920, 1080, 720)).toEqual({ width: 1280, height: 720 });
  expect(fitVideo(640, 480, 1080)).toEqual({ width: 640, height: 480 });
  expect(fitVideo(1080, 1920, 1080)).toEqual({ width: 608, height: 1080 });
});
