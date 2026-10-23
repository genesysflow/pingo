#[cfg(not(feature = "webrtc-apm"))]
use crate::FRAME_SAMPLES;
use crate::{MonoFrame, Result};

/// State is kept per microphone; callers feed the actual rendered speaker mix.
pub struct AudioProcessor {
    #[cfg(feature = "webrtc-apm")]
    processor: webrtc_audio_processing::Processor,
    #[cfg(not(feature = "webrtc-apm"))]
    previous_input: f32,
    #[cfg(not(feature = "webrtc-apm"))]
    previous_output: f32,
    #[cfg(not(feature = "webrtc-apm"))]
    gain: f32,
    agc: bool,
    echo: bool,
}

impl AudioProcessor {
    pub fn new(agc: bool, echo: bool) -> Result<Self> {
        let mut this = Self {
            #[cfg(feature = "webrtc-apm")]
            processor: webrtc_audio_processing::Processor::new(crate::SAMPLE_RATE)
                .map_err(|e| crate::Error::Processing(e.to_string()))?,
            #[cfg(not(feature = "webrtc-apm"))]
            previous_input: 0.0,
            #[cfg(not(feature = "webrtc-apm"))]
            previous_output: 0.0,
            #[cfg(not(feature = "webrtc-apm"))]
            gain: 1.0,
            agc,
            echo,
        };
        this.configure(agc, echo);
        Ok(this)
    }

    pub fn has_aec(&self) -> bool {
        cfg!(feature = "webrtc-apm") && self.echo
    }

    pub fn configure(&mut self, agc: bool, echo: bool) {
        self.agc = agc;
        self.echo = echo;
        #[cfg(feature = "webrtc-apm")]
        {
            use webrtc_audio_processing::config::*;
            self.processor.set_config(Config {
                high_pass_filter: Some(HighPassFilter::default()),
                echo_canceller: echo.then(EchoCanceller::default),
                gain_controller: agc.then(|| {
                    GainController::GainController2(GainController2 {
                        adaptive_digital: Some(AdaptiveDigital::default()),
                        ..Default::default()
                    })
                }),
                ..Default::default()
            });
        }
    }

    pub fn render(&mut self, frame: &MonoFrame) -> Result<()> {
        #[cfg(feature = "webrtc-apm")]
        self.processor
            .analyze_render_frame([frame.as_slice()])
            .map_err(|e| crate::Error::Processing(e.to_string()))?;
        #[cfg(not(feature = "webrtc-apm"))]
        let _ = frame;
        Ok(())
    }

    pub fn capture(&mut self, frame: &mut MonoFrame) -> Result<()> {
        #[cfg(feature = "webrtc-apm")]
        self.processor
            .process_capture_frame([frame.as_mut_slice()])
            .map_err(|e| crate::Error::Processing(e.to_string()))?;
        #[cfg(not(feature = "webrtc-apm"))]
        {
            // One-pole DC blocker, -3 dB at approximately 80 Hz at 48 kHz.
            let coefficient = (-2.0 * std::f32::consts::PI * 80.0 / 48_000.0).exp();
            for sample in frame.iter_mut() {
                let output = *sample - self.previous_input + coefficient * self.previous_output;
                self.previous_input = *sample;
                self.previous_output = output;
                *sample = output;
            }
            if self.agc {
                let power = super::level::rms(frame);
                // Do not amplify the noise floor during silence.
                let target = if power > 0.003 {
                    (0.12 / power).clamp(0.25, 8.0)
                } else {
                    1.0
                };
                let old_gain = self.gain;
                self.gain += (target - self.gain) * if target < self.gain { 0.15 } else { 0.01 };
                for (i, sample) in frame.iter_mut().enumerate() {
                    let gain = old_gain + (self.gain - old_gain) * i as f32 / FRAME_SAMPLES as f32;
                    *sample = (*sample * gain).clamp(-0.98, 0.98);
                }
            }
        }
        Ok(())
    }
}
