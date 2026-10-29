import { FilesetResolver, ImageSegmenter } from '@mediapipe/tasks-vision';
import simdLoader from 'mediapipe-wasm/vision_wasm_internal.js?url';
import simdBinary from 'mediapipe-wasm/vision_wasm_internal.wasm?url';
import scalarLoader from 'mediapipe-wasm/vision_wasm_nosimd_internal.js?url';
import scalarBinary from 'mediapipe-wasm/vision_wasm_nosimd_internal.wasm?url';

export async function createSegmenter() {
  const simd = await FilesetResolver.isSimdSupported();
  return ImageSegmenter.createFromOptions(
    {
      wasmLoaderPath: simd ? simdLoader : scalarLoader,
      wasmBinaryPath: simd ? simdBinary : scalarBinary,
    },
    {
      baseOptions: {
        modelAssetPath: `${import.meta.env.BASE_URL}background-blur/selfie_segmenter.tflite`,
        delegate: 'CPU',
      },
      runningMode: 'VIDEO',
      outputCategoryMask: false,
      outputConfidenceMasks: true,
    },
  );
}
