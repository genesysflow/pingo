use crate::{Error, Result, SAMPLE_RATE};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Profile {
    #[default]
    Voice,
    Studio,
}
impl Profile {
    pub fn channels(self) -> usize {
        if self == Self::Voice { 1 } else { 2 }
    }
    fn opus_channels(self) -> opus::Channels {
        if self == Self::Voice {
            opus::Channels::Mono
        } else {
            opus::Channels::Stereo
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PacketDuration {
    LowLatency10,
    #[default]
    Default20,
}
impl PacketDuration {
    pub fn samples(self) -> usize {
        if self == Self::LowLatency10 { 480 } else { 960 }
    }
    pub fn millis(self) -> u32 {
        self.samples() as u32 / 48
    }
}

#[derive(Clone, Debug)]
pub struct EncodedPacket {
    pub sequence: u16,
    pub timestamp: u32,
    pub duration: PacketDuration,
    pub payload: Vec<u8>,
    pub audio_level: u8,
    pub speaking: bool,
}

pub struct Encoder {
    inner: opus::Encoder,
    pub profile: Profile,
    pub duration: PacketDuration,
}
impl Encoder {
    pub fn new(profile: Profile, duration: PacketDuration) -> Result<Self> {
        let mut inner = opus::Encoder::new(
            SAMPLE_RATE,
            profile.opus_channels(),
            if profile == Profile::Voice {
                opus::Application::Voip
            } else {
                opus::Application::Audio
            },
        )?;
        inner.set_bitrate(opus::Bitrate::Bits(if profile == Profile::Voice {
            64_000
        } else {
            192_000
        }))?;
        inner.set_vbr(true)?;
        inner.set_signal(if profile == Profile::Voice {
            opus::Signal::Voice
        } else {
            opus::Signal::Music
        })?;
        inner.set_inband_fec(profile == Profile::Voice)?;
        inner.set_dtx(profile == Profile::Voice)?;
        inner.set_packet_loss_perc(5)?;
        Ok(Self {
            inner,
            profile,
            duration,
        })
    }
    pub fn feedback(&mut self, loss_percent: f32) -> Result<()> {
        self.inner
            .set_packet_loss_perc(loss_percent.round().clamp(0.0, 100.0) as i32)?;
        Ok(())
    }
    pub fn lookahead_samples(&mut self) -> Result<usize> {
        Ok(self.inner.get_lookahead()? as usize)
    }
    pub fn encode(&mut self, pcm: &[f32]) -> Result<Vec<u8>> {
        if pcm.len() != self.duration.samples() * self.profile.channels() {
            return Err(Error::Invalid(
                "PCM packet length does not match codec profile/duration",
            ));
        }
        let mut payload = vec![0; 4000];
        let len = self.inner.encode_float(pcm, &mut payload)?;
        payload.truncate(len);
        Ok(payload)
    }
}

pub struct Decoder {
    inner: opus::Decoder,
    pub channels: usize,
}
impl Decoder {
    pub fn new(profile: Profile) -> Result<Self> {
        Ok(Self {
            inner: opus::Decoder::new(SAMPLE_RATE, profile.opus_channels())?,
            channels: profile.channels(),
        })
    }
    /// An absent payload invokes Opus PLC. FEC reads the *next* packet without consuming it.
    pub fn decode(&mut self, packet: Option<&[u8]>, samples: usize, fec: bool) -> Result<Vec<f32>> {
        let mut pcm = vec![0.0; samples * self.channels];
        let len = self
            .inner
            .decode_float(packet.unwrap_or(&[]), &mut pcm, fec)?;
        pcm.truncate(len * self.channels);
        Ok(pcm)
    }
}
