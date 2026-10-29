import { FilesetResolver, ImageSegmenter } from '@mediapipe/tasks-vision';
import simdLoader from 'mediapipe-wasm/vision_wasm_internal.js?url';
import simdBinary from 'mediapipe-wasm/vision_wasm_internal.wasm?url';
import scalarLoader from 'mediapipe-wasm/vision_wasm_nosimd_internal.js?url';
import scalarBinary from 'mediapipe-wasm/vision_wasm_nosimd_internal.wasm?url';
import type { BlurDelegate } from './blur-worker-protocol';

export async function createSegmenter(baseUrl: string, delegate: BlurDelegate) {
  const simd = await FilesetResolver.isSimdSupported();
  return ImageSegmenter.createFromOptions(
    {
      wasmLoaderPath: new URL(simd ? simdLoader : scalarLoader, baseUrl).href,
      wasmBinaryPath: new URL(simd ? simdBinary : scalarBinary, baseUrl).href,
    },
    {
      baseOptions: {
        modelAssetPath: new URL('background-blur/selfie_segmenter.tflite', baseUrl).href,
        delegate,
      },
      canvas: new OffscreenCanvas(1, 1),
      runningMode: 'VIDEO',
      outputCategoryMask: false,
      outputConfidenceMasks: true,
    },
  );
}
