# pingo-denoise

DeepFilterNet3 noise suppression at 48 kHz, in 10 ms (480-sample) frames. The same
code runs natively in `pingo-media` (desktop) and as WebAssembly in the browser's
call microphone (`apps/web/src/lib/call/denoise-worklet.ts`), so Gentle and Strong
sound the same on both.

```rust
let mut filter = pingo_denoise::DeepFilter::new(Strength::Gentle)?;
filter.process(&mut frame)?; // in place; output lags input by delay_samples() = 1440 (30 ms)
filter.set_strength(Strength::Strong); // applies on the next frame, no reset
```

- **Gentle** caps attenuation at 18 dB so the room keeps a little air.
- **Strong** removes as much as the model can, plus upstream's light post filter (β 0.02).
- The model is parsed once per process (≈250 ms native, ≈450 ms WASM); call
  `preload()` early. Each filter then gets fresh state over the shared plans.

## Where the code comes from

`src/df/` is vendored from [Rikorose/DeepFilterNet](https://github.com/Rikorose/DeepFilterNet)
`libDF` at commit `d375b2d` (MIT OR Apache-2.0, see `LICENSE-MIT` and `LICENSE-APACHE`).
`models/DeepFilterNet3_onnx.tar.gz` is the upstream DeepFilterNet3 model from the same commit.
It's vendored rather than a git dependency because the repository is ~100 MB and
the crates.io `deep_filter` 0.2.5 has no inference runner. Changes from upstream:

- Only the STFT/ERB DSP (`lib.rs` → `df/mod.rs`) and the tract runner (`tract.rs`) are kept.
- `DfModels` holds the optimized graphs behind `Arc`; `DfTract::from_models` builds
  per-filter state. Upstream's `#[derive(Clone)]` on `DfTract` shared its `Rc`
  scratch tensors between clones (and mutated them through a raw pointer), so it is removed.
- Logging goes through `tracing`; feature-gated defaults and timing code are removed.
- tract is pinned to `=0.21.4`, which the runner was written against (0.21.6 renamed
  `symbol_table`; 0.21.8+ moved to ndarray 0.16).
- Runtime thresholds are upstream's CLI defaults (`-15/35/35` dB). The library defaults
  (`-10/30/20`) skip deep filtering above 20 dB SNR, which halved SI-SDR on real speech.

## Measurements

On DeepFilterNet's own sample assets (real speech `clean_freesound_33711` mixed with
noise `2530` at 0 dB SNR), SI-SDR went from 0.0 dB to **10.4 dB**, against 4.3 dB for
RNNoise (Strong) and 2.6 dB for RNNoise (Gentle, the previous default). Clean speech passes at
39 dB SI-SDR (RNNoise Strong: 20 dB). The low-latency DFN3 variant (10 ms) was rejected:
it removed only 1–12 dB of noise between words.

CPU on a Ryzen 7 5800X3D: 0.15 ms per 10 ms frame native (1.5% of a core), 1.4 ms in
WASM with SIMD (V8). The size-optimized WASM is 17.9 MB (9.9 MB gzipped; the model is 8 MB of it).

## Building for the browser

```sh
rustup target add wasm32-unknown-unknown
cd apps/web && bun run denoise   # → public/denoise/pingo-denoise.wasm
```

`bun run dev` builds it when it can (and warns otherwise); `bun run build` requires it.
The exports are plain C ABI (`src/wasm.rs`), so there is no wasm-bindgen glue.

## Tests

`cargo test -p pingo-denoise` checks delay, speech preservation, the Gentle cap, Strong's
reduction, noisy-speech improvement, live strength changes and per-filter state
isolation across threads. In dev builds tract is compiled with `opt-level = 3` and no
debug assertions (root `Cargo.toml`), because one of its debug checks rejects these graphs.
