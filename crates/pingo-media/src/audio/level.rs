use crate::MonoFrame;

pub fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|x| x * x).sum::<f32>() / samples.len().max(1) as f32).sqrt()
}

/// RFC 6464 uses negative dBov: silence is 127 and full-scale is zero.
pub fn dbov(samples: &[f32]) -> u8 {
    (-20.0 * rms(samples).max(1e-10).log10())
        .round()
        .clamp(0.0, 127.0) as u8
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Gate {
    Open,
    #[default]
    VoiceActivation,
    PushToTalk {
        pressed: bool,
    },
    Muted,
}

pub struct VoiceActivity {
    pub threshold_dbov: u8,
    pub hangover_frames: u16,
    remaining: u16,
    gain: f32,
}

impl Default for VoiceActivity {
    fn default() -> Self {
        Self {
            threshold_dbov: 48,
            hangover_frames: 20,
            remaining: 0,
            gain: 0.0,
        }
    }
}

impl VoiceActivity {
    pub fn detect(&mut self, samples: &[f32]) -> bool {
        if dbov(samples) < self.threshold_dbov {
            self.remaining = self.hangover_frames;
            true
        } else if self.remaining > 0 {
            self.remaining -= 1;
            true
        } else {
            false
        }
    }

    /// A short ramp avoids clicks when a PTT key or the VAD gate changes.
    pub fn apply(&mut self, samples: &mut MonoFrame, open: bool) {
        let target = if open { 1.0 } else { 0.0 };
        for s in samples {
            self.gain += (target - self.gain).clamp(-1.0 / 240.0, 1.0 / 240.0);
            *s *= self.gain;
        }
    }
}
