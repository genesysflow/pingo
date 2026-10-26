use crate::{
    FRAME_SAMPLES, MonoFrame, Result, StereoFrame,
    audio::{
        apm::AudioProcessor,
        codec::{EncodedPacket, Encoder, PacketDuration, Profile},
        denoise::{self, NoiseLevel, NoiseSuppressor},
        jitter::{JitterBuffer, JitterStats},
        level::{self, Gate, VoiceActivity},
        mixer::{Mixer, ParticipantMix},
    },
};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct AudioSettings {
    pub profile: Profile,
    pub packet_duration: PacketDuration,
    pub noise_level: NoiseLevel,
    pub agc: bool,
    pub echo_cancel: bool,
    pub gate: Gate,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub per_user_volumes: HashMap<String, f32>,
    pub per_user_pan: HashMap<String, f32>,
    pub normalize: bool,
    pub deafened: bool,
}
impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            profile: Profile::Voice,
            packet_duration: PacketDuration::Default20,
            noise_level: NoiseLevel::Gentle,
            agc: true,
            echo_cancel: true,
            gate: Gate::VoiceActivation,
            input_device: None,
            output_device: None,
            per_user_volumes: HashMap::new(),
            per_user_pan: HashMap::new(),
            normalize: true,
            deafened: false,
        }
    }
}

pub struct SendPipeline {
    settings: AudioSettings,
    apm: AudioProcessor,
    denoise: Box<dyn NoiseSuppressor>,
    vad: VoiceActivity,
    encoder: Encoder,
    pending: Vec<f32>,
    sequence: u16,
    timestamp: u32,
    pending_timestamp: u32,
    pending_speaking: bool,
    expected_loss: f32,
}
impl SendPipeline {
    pub fn new(settings: AudioSettings) -> Result<Self> {
        Ok(Self {
            apm: AudioProcessor::new(settings.agc, settings.echo_cancel)?,
            denoise: denoise::default_suppressor(),
            vad: VoiceActivity::default(),
            encoder: Encoder::new(settings.profile, settings.packet_duration)?,
            settings,
            pending: Vec::with_capacity(1920),
            sequence: 0,
            timestamp: 0,
            pending_timestamp: 0,
            pending_speaking: false,
            expected_loss: 5.0,
        })
    }
    pub fn settings(&self) -> &AudioSettings {
        &self.settings
    }
    pub fn has_aec(&self) -> bool {
        self.apm.has_aec() && self.settings.profile == Profile::Voice
    }
    pub fn set_settings(&mut self, settings: AudioSettings) -> Result<()> {
        if settings.profile != self.settings.profile
            || settings.packet_duration != self.settings.packet_duration
        {
            let mut encoder = Encoder::new(settings.profile, settings.packet_duration)?;
            encoder.feedback(self.expected_loss)?;
            self.encoder = encoder;
            self.pending.clear();
            self.pending_speaking = false;
        }
        // Strength changes apply in place; stale state from another profile or
        // from before an Off period would smear the first frames instead.
        if settings.profile != self.settings.profile
            || (self.settings.noise_level == NoiseLevel::Off
                && settings.noise_level != NoiseLevel::Off)
        {
            self.denoise = denoise::default_suppressor();
        }
        self.apm.configure(settings.agc, settings.echo_cancel);
        self.settings = settings;
        Ok(())
    }
    pub fn feedback(&mut self, loss_percent: f32) -> Result<()> {
        self.expected_loss = loss_percent;
        self.encoder.feedback(loss_percent)
    }
    /// Includes one conservative capture and playback quantum; excludes device,
    /// resampler, operating-system and network delay. Jitter is supplied by receiver.
    pub fn latency_budget_ms(&mut self, jitter_ms: f32) -> Result<f32> {
        let denoise = if self.settings.profile == Profile::Voice
            && self.settings.noise_level != NoiseLevel::Off
        {
            self.denoise.delay_samples() as f32 / 48.0
        } else {
            0.0
        };
        Ok(10.0
            + self.settings.packet_duration.millis() as f32
            + denoise
            + self.encoder.lookahead_samples()? as f32 / 48.0
            + jitter_ms
            + 10.0)
    }
    pub fn set_stream_delay_ms(&mut self, delay_ms: Option<u16>) {
        self.apm.set_stream_delay_ms(delay_ms);
    }
    pub fn render_reference(&mut self, frame: &StereoFrame) -> Result<()> {
        if self.settings.profile == Profile::Voice {
            self.apm.render_stereo(frame)?;
        }
        Ok(())
    }
    pub fn process_mono(&mut self, frame: &MonoFrame) -> Result<Option<EncodedPacket>> {
        self.process(&frame.map(|s| [s, s]))
    }
    pub fn process(&mut self, input: &StereoFrame) -> Result<Option<EncodedPacket>> {
        let mut mono = input.map(|s| (s[0] + s[1]) * 0.5);
        let voice = self.settings.profile == Profile::Voice;
        if voice {
            self.apm.capture(&mut mono)?;
            self.denoise.process(&mut mono, self.settings.noise_level);
        }
        let active = if voice {
            self.vad.detect(&mono)
        } else {
            self.vad
                .detect(&input.map(|s| ((s[0] * s[0] + s[1] * s[1]) * 0.5).sqrt()))
        };
        let open = match self.settings.gate {
            Gate::Open => true,
            Gate::VoiceActivation => active || !voice,
            Gate::PushToTalk { pressed } => pressed,
            Gate::Muted => false,
        };
        let mut envelope = [1.0; FRAME_SAMPLES];
        self.vad.apply(&mut envelope, open);
        if self.pending.is_empty() {
            self.pending_timestamp = self.timestamp;
        }
        self.timestamp = self.timestamp.wrapping_add(FRAME_SAMPLES as u32);
        self.pending_speaking |= active && open;
        for (i, sample) in mono.iter().enumerate() {
            if voice {
                self.pending.push(sample * envelope[i]);
            } else {
                self.pending.push(input[i][0] * envelope[i]);
                self.pending.push(input[i][1] * envelope[i]);
            }
        }
        if self.pending.len()
            < self.settings.packet_duration.samples() * self.settings.profile.channels()
        {
            return Ok(None);
        }
        let payload = self.encoder.encode(&self.pending)?;
        let packet = EncodedPacket {
            sequence: self.sequence,
            timestamp: self.pending_timestamp,
            duration: self.settings.packet_duration,
            audio_level: level::dbov(&self.pending),
            speaking: self.pending_speaking,
            payload,
        };
        self.sequence = self.sequence.wrapping_add(1);
        self.pending.clear();
        self.pending_speaking = false;
        Ok(Some(packet))
    }
}

pub struct ReceivePipeline {
    settings: AudioSettings,
    participants: HashMap<String, JitterBuffer>,
    mixer: Mixer,
}
impl ReceivePipeline {
    pub fn new(settings: AudioSettings) -> Self {
        Self {
            settings,
            participants: HashMap::new(),
            mixer: Mixer::default(),
        }
    }
    pub fn set_settings(&mut self, settings: AudioSettings) {
        self.settings = settings;
    }
    /// A stereo decoder accepts both mono Voice and stereo Studio packets, allowing
    /// remote profile switches without a signaling contract change.
    pub fn add_participant(&mut self, id: String) -> Result<()> {
        if let std::collections::hash_map::Entry::Vacant(entry) = self.participants.entry(id) {
            entry.insert(JitterBuffer::new(Profile::Studio)?);
        }
        Ok(())
    }
    pub fn remove_participant(&mut self, id: &str) {
        self.participants.remove(id);
        self.mixer.remove(id);
    }
    pub fn push(&mut self, id: &str, packet: EncodedPacket, arrival_ms: f64) -> Result<bool> {
        if let Some(buffer) = self.participants.get_mut(id) {
            buffer.push(packet, arrival_ms)
        } else {
            Ok(false)
        }
    }
    pub fn stats(&self) -> HashMap<String, JitterStats> {
        self.participants
            .iter()
            .map(|(id, b)| (id.clone(), b.stats()))
            .collect()
    }
    pub fn render(&mut self, now_ms: f64) -> Result<StereoFrame> {
        let mut rendered = Vec::with_capacity(self.participants.len());
        for (id, buffer) in &mut self.participants {
            rendered.push((
                id.as_str(),
                buffer.render(now_ms)?,
                buffer.channels(),
                ParticipantMix {
                    volume: self
                        .settings
                        .per_user_volumes
                        .get(id)
                        .copied()
                        .unwrap_or(1.0),
                    pan: self.settings.per_user_pan.get(id).copied(),
                    normalize: self.settings.normalize && self.settings.profile != Profile::Studio,
                },
            ));
        }
        self.mixer.deafened = self.settings.deafened;
        Ok(self.mixer.mix(
            rendered
                .iter()
                .map(|(id, pcm, channels, settings)| (*id, pcm.as_slice(), *channels, *settings)),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Great,
    Ok,
    Poor,
}
impl std::fmt::Display for Quality {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Great => "Great",
            Self::Ok => "OK",
            Self::Poor => "Poor",
        })
    }
}
#[derive(Clone, Copy, Debug)]
pub struct QualityReport {
    pub quality: Quality,
    pub loss_percent: f32,
    pub jitter_ms: f32,
    pub rtt_ms: f32,
    pub delay_ms: f32,
}
impl QualityReport {
    pub fn new(loss_percent: f32, jitter_ms: f32, rtt_ms: f32, delay_ms: f32) -> Self {
        let valid = [loss_percent, jitter_ms, rtt_ms, delay_ms]
            .iter()
            .all(|x| x.is_finite() && *x >= 0.0);
        let quality = if !valid
            || loss_percent >= 8.0
            || jitter_ms >= 60.0
            || rtt_ms >= 250.0
            || delay_ms >= 150.0
        {
            Quality::Poor
        } else if loss_percent >= 2.0 || jitter_ms >= 20.0 || rtt_ms >= 120.0 || delay_ms >= 80.0 {
            Quality::Ok
        } else {
            Quality::Great
        };
        Self {
            quality,
            loss_percent,
            jitter_ms,
            rtt_ms,
            delay_ms,
        }
    }
    pub fn from_jitter(stats: JitterStats, rtt_ms: f32) -> Self {
        Self::new(
            stats.loss_percent,
            stats.jitter_ms,
            rtt_ms,
            stats.current_delay_ms,
        )
    }
}
impl std::fmt::Display for QualityReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (loss {:.1}%, jitter {:.0} ms, RTT {:.0} ms, playout {:.0} ms)",
            self.quality, self.loss_percent, self.jitter_ms, self.rtt_ms, self.delay_ms
        )
    }
}
