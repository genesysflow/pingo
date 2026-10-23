use crate::{FRAME_SAMPLES, MonoFrame};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NoiseLevel {
    Off,
    #[default]
    Gentle,
    Strong,
}

pub trait NoiseSuppressor: Send {
    /// Returns a speech probability; frames are normalized mono float PCM.
    fn process(&mut self, frame: &mut MonoFrame, level: NoiseLevel) -> f32;
    fn delay_samples(&self) -> usize;
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

impl NoiseSuppressor for RnNoise {
    fn process(&mut self, frame: &mut MonoFrame, level: NoiseLevel) -> f32 {
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
    fn delay_samples(&self) -> usize {
        FRAME_SAMPLES
    }
}

/// Explicit failure avoids silently advertising RNNoise as DeepFilterNet.
#[cfg(feature = "deepfilter")]
pub fn deepfilter() -> crate::Result<Box<dyn NoiseSuppressor>> {
    let _dsp = df::DFState::new(48_000, 960, 480, 32, 2);
    Err(crate::Error::Processing(
        "deep_filter 0.2.5 provides FFT/ERB primitives only; no inference runner or bundled model. Use RNNoise until a model is packaged offline.".into(),
    ))
}
