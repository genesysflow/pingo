use crate::{FRAME_SAMPLES, StereoFrame};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub struct ParticipantMix {
    pub volume: f32,
    pub pan: Option<f32>,
    pub normalize: bool,
}
impl Default for ParticipantMix {
    fn default() -> Self {
        Self {
            volume: 1.0,
            pan: None,
            normalize: true,
        }
    }
}
#[derive(Default)]
struct Loudness {
    energy: f32,
    gain: f32,
}

#[derive(Default)]
pub struct Mixer {
    pub deafened: bool,
    state: HashMap<String, Loudness>,
}
impl Mixer {
    pub fn remove(&mut self, id: &str) {
        self.state.remove(id);
    }
    /// Rough 400 ms loudness integration with gated, slow gain. This is not a
    /// standards-compliant R128 meter: K-weighting and true-peak metering are future work.
    pub fn mix<'a>(
        &mut self,
        inputs: impl IntoIterator<Item = (&'a str, &'a [f32], usize, ParticipantMix)>,
    ) -> StereoFrame {
        let mut output = [[0.0; 2]; FRAME_SAMPLES];
        for (id, pcm, channels, settings) in inputs {
            if !(1..=2).contains(&channels) || pcm.len() != FRAME_SAMPLES * channels {
                continue;
            }
            let state = self.state.entry(id.to_owned()).or_insert(Loudness {
                energy: 0.0,
                gain: 1.0,
            });
            let energy = pcm.iter().map(|x| x * x).sum::<f32>() / pcm.len() as f32;
            state.energy += 0.025 * (energy - state.energy);
            let target = if state.energy > 1e-5 {
                (0.1 / state.energy.sqrt()).clamp(0.25, 4.0)
            } else {
                1.0
            };
            let old_gain = state.gain;
            state.gain += (target - state.gain) * if target < state.gain { 0.02 } else { 0.002 };
            let pan = settings
                .pan
                .map(|p| (p.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4);
            for (i, frame) in output.iter_mut().enumerate() {
                let gain = settings.volume.clamp(0.0, 4.0)
                    * if settings.normalize {
                        old_gain + (state.gain - old_gain) * i as f32 / FRAME_SAMPLES as f32
                    } else {
                        1.0
                    };
                let left = pcm[i * channels];
                let right = pcm[i * channels + channels - 1];
                if let Some(angle) = pan {
                    let mono = (left + right) * 0.5;
                    frame[0] += mono * angle.cos() * gain;
                    frame[1] += mono * angle.sin() * gain;
                } else {
                    frame[0] += left * gain;
                    frame[1] += right * gain;
                }
            }
        }
        for frame in &mut output {
            for sample in frame {
                // Unity below the knee preserves quiet material, with bounded peaks.
                let a = sample.abs();
                *sample = if self.deafened {
                    0.0
                } else if a <= 0.8 {
                    *sample
                } else {
                    sample.signum() * (0.8 + 0.2 * ((a - 0.8) / 0.2).tanh())
                };
            }
        }
        output
    }
}
