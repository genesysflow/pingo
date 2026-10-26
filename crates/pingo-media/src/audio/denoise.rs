use crate::{FRAME_SAMPLES, MonoFrame};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NoiseLevel {
    Off,
    #[default]
    Gentle,
    Strong,
}

pub trait NoiseSuppressor: Send {
    /// Frames are normalized mono float PCM. `Off` must leave the frame untouched.
    fn process(&mut self, frame: &mut MonoFrame, level: NoiseLevel);
    fn delay_samples(&self) -> usize;
}

/// The suppressor calls use: DeepFilterNet3 when built with the `deepfilter`
/// feature and its model loads, RNNoise otherwise.
pub fn default_suppressor() -> Box<dyn NoiseSuppressor> {
    #[cfg(feature = "deepfilter")]
    match DeepFilter::new() {
        Ok(filter) => return Box::new(filter),
        Err(error) => tracing::warn!(%error, "DeepFilterNet unavailable, using RNNoise"),
    }
    Box::new(RnNoise::default())
}

/// Parses the DeepFilterNet model (a few hundred ms) ahead of the first call,
/// so joining does not stall the media loop. A no-op without `deepfilter`.
pub fn preload() {
    #[cfg(feature = "deepfilter")]
    if let Err(error) = pingo_denoise::preload() {
        tracing::warn!(%error, "could not preload DeepFilterNet");
    }
}

/// DeepFilterNet3 at 48 kHz: much better than RNNoise on non-stationary noise
/// and keeps speech intact, for 30 ms of delay instead of 10 ms.
#[cfg(feature = "deepfilter")]
pub struct DeepFilter {
    filter: pingo_denoise::DeepFilter,
}

#[cfg(feature = "deepfilter")]
impl DeepFilter {
    pub fn new() -> pingo_denoise::Result<Self> {
        Ok(Self {
            filter: pingo_denoise::DeepFilter::new(pingo_denoise::Strength::Gentle)?,
        })
    }
}

#[cfg(feature = "deepfilter")]
impl NoiseSuppressor for DeepFilter {
    fn process(&mut self, frame: &mut MonoFrame, level: NoiseLevel) {
        let strength = match level {
            NoiseLevel::Off => return,
            NoiseLevel::Gentle => pingo_denoise::Strength::Gentle,
            NoiseLevel::Strong => pingo_denoise::Strength::Strong,
        };
        self.filter.set_strength(strength);
        if let Err(error) = self.filter.process(frame) {
            // The frame is left as captured; dropping audio would be worse.
            tracing::warn!(%error, "noise suppression frame failed");
        }
    }
    fn delay_samples(&self) -> usize {
        self.filter.delay_samples()
    }
}

pub struct RnNoise {
    state: Box<nnnoiseless::DenoiseState<'static>>,
    dry: MonoFrame,
}

impl Default for RnNoise {
    fn default() -> Self {
        Self {
            state: nnnoiseless::DenoiseState::new(),
            dry: [0.0; FRAME_SAMPLES],
        }
    }
}

impl RnNoise {
    /// Returns RNNoise's voice probability for the frame (0 when `Off`).
    pub fn process_with_probability(&mut self, frame: &mut MonoFrame, level: NoiseLevel) -> f32 {
        if level == NoiseLevel::Off {
            return 0.0;
        }
        let input = frame.map(|x| x * 32768.0);
        let mut wet = [0.0; FRAME_SAMPLES];
        let probability = self.state.process_frame(&mut wet, &input);
        let mix = if level == NoiseLevel::Gentle {
            0.5
        } else {
            1.0
        };
        // RNNoise has one frame of lookahead. Align the dry signal before mixing,
        // otherwise Gentle produces comb filtering and a double attack.
        for i in 0..FRAME_SAMPLES {
            let dry = self.dry[i];
            self.dry[i] = frame[i];
            frame[i] = (dry * (1.0 - mix) + wet[i] / 32768.0 * mix).clamp(-1.0, 1.0);
        }
        probability
    }
}

impl NoiseSuppressor for RnNoise {
    fn process(&mut self, frame: &mut MonoFrame, level: NoiseLevel) {
        self.process_with_probability(frame, level);
    }
    fn delay_samples(&self) -> usize {
        FRAME_SAMPLES
    }
}
