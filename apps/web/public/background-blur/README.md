# Selfie segmentation model

`selfie_segmenter.tflite` is Google's MediaPipe Selfie Segmenter float16 model, distributed under Apache-2.0.

- Source: https://storage.googleapis.com/mediapipe-models/image_segmenter/selfie_segmenter/float16/latest/selfie_segmenter.tflite
- Model information: https://ai.google.dev/edge/mediapipe/solutions/vision/image_segmenter
- SHA-256: `191ac9529ae506ee0beefa6b2c945a172dab9d07d1e802a290a4e4038226658b`

The runtime is pinned in `package.json`; Vite bundles its SIMD and non-SIMD WASM assets from that package.

`worker.js` bootstraps the bundled segmentation module in a classic worker. MediaPipe's WASM loader uses
`importScripts`, which is unavailable in a module worker. Loading our module from a classic worker supports
that loader without evaluating script strings or moving inference back onto the UI thread. All worker,
runtime and model URLs stay on the Pingo origin and honor the Vite base URL.
