//! DeepFilterNet3 speech enhancement at 48 kHz.
//!
//! One implementation serves the desktop media engine and the browser (built
//! for `wasm32-unknown-unknown`, see `src/wasm.rs`), so both sides of a call
//! sound the same. The runner in `df/` is vendored from
//! <https://github.com/Rikorose/DeepFilterNet> (see `VENDORED.md`).

#[allow(
    clippy::all,
    dead_code,
    unused_imports,
    mismatched_lifetime_syntaxes,
    unsafe_op_in_unsafe_fn
)]
mod df;
#[cfg(target_arch = "wasm32")]
mod wasm;

use df::tract::{DfModels, DfParams, DfTract, RuntimeParams};
use ndarray::{ArrayView2, ArrayViewMut2};
use std::sync::OnceLock;

/// Samples per processing step: 10 ms at 48 kHz.
pub const HOP: usize = 480;
pub const SAMPLE_RATE: u32 = 48_000;

/// Bundled DeepFilterNet3 model (encoder, ERB and DF decoders, config).
static MODEL: &[u8] = include_bytes!("../models/DeepFilterNet3_onnx.tar.gz");

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not load the noise suppression model: {0}")]
    Model(String),
    #[error("noise suppression failed: {0}")]
    Process(String),
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strength {
    /// Caps attenuation so the room keeps a little air; speech is never pumped.
    Gentle,
    /// Removes as much noise as the model can, plus a light post filter.
    Strong,
}
impl Strength {
    fn attenuation_limit_db(self) -> f32 {
        match self {
            Strength::Gentle => 18.0,
            Strength::Strong => 100.0,
        }
    }
    fn post_filter_beta(self) -> f32 {
        match self {
            Strength::Gentle => 0.0,
            Strength::Strong => 0.02,
        }
    }
}

/// Parsing and optimizing the ONNX graphs takes a few hundred milliseconds, so
/// it happens once per process; every suppressor gets fresh state over the
/// shared plans.
static MODELS: OnceLock<std::result::Result<DfModels, String>> = OnceLock::new();

fn runtime() -> RuntimeParams {
    // Upstream's CLI thresholds. The library defaults skip the deep-filter
    // stage above 20 dB local SNR, which audibly smears speech in mild noise.
    RuntimeParams::default_with_ch(1).with_thresholds(-15.0, 35.0, 35.0)
}

fn models() -> Result<&'static DfModels> {
    MODELS
        .get_or_init(|| {
            let params = DfParams::from_bytes(MODEL).map_err(|e| format!("{e:#}"))?;
            DfModels::new(params, &runtime()).map_err(|e| format!("{e:#}"))
        })
        .as_ref()
        .map_err(|e| Error::Model(e.clone()))
}

/// Loads the model ahead of time so the first call does not stall an audio thread.
pub fn preload() -> Result<()> {
    models().map(drop)
}

pub struct DeepFilter {
    model: DfTract,
    strength: Strength,
    output: [f32; HOP],
}

// SAFETY: `DfTract` is `!Send` only because tract keeps scratch tensors in
// `Rc`s and op states behind `Box<dyn OpState>`. Every one of those is created
// by `DfTract::from_models` for this instance and never handed out (the shared
// plans are `Arc`), so moving the whole filter to another thread cannot leave
// a reference count or state shared across threads.
unsafe impl Send for DeepFilter {}

impl DeepFilter {
    pub fn new(strength: Strength) -> Result<Self> {
        let mut filter = Self {
            model: DfTract::from_models(models()?, &runtime())
                .map_err(|e| Error::Model(format!("{e:#}")))?,
            strength,
            output: [0.0; HOP],
        };
        filter.apply_strength();
        Ok(filter)
    }

    pub fn strength(&self) -> Strength {
        self.strength
    }

    /// Changes take effect on the next frame without resetting model state.
    pub fn set_strength(&mut self, strength: Strength) {
        if strength != self.strength {
            self.strength = strength;
            self.apply_strength();
        }
    }

    fn apply_strength(&mut self) {
        self.model
            .set_atten_lim(self.strength.attenuation_limit_db());
        self.model.set_pf_beta(self.strength.post_filter_beta());
    }

    /// Enhances one 10 ms frame of normalized mono PCM in place and returns the
    /// model's local SNR estimate in dB. Output lags input by [`Self::delay_samples`].
    pub fn process(&mut self, frame: &mut [f32; HOP]) -> Result<f32> {
        let input = ArrayView2::from_shape((1, HOP), &frame[..])
            .map_err(|e| Error::Process(e.to_string()))?;
        let output = ArrayViewMut2::from_shape((1, HOP), &mut self.output[..])
            .map_err(|e| Error::Process(e.to_string()))?;
        let lsnr = self
            .model
            .process(input, output)
            .map_err(|e| Error::Process(format!("{e:#}")))?;
        frame.copy_from_slice(&self.output);
        Ok(lsnr)
    }

    /// STFT overlap plus the model's lookahead: 30 ms for DeepFilterNet3.
    pub fn delay_samples(&self) -> usize {
        self.model.fft_size - self.model.hop_size + self.model.lookahead * self.model.hop_size
    }
}
