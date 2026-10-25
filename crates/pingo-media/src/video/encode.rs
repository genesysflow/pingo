use super::{Result, VideoError, color::I420Frame};
use openh264::{
    OpenH264API, Timestamp,
    encoder::{
        BitRate, EncoderConfig, FrameRate, FrameType, IntraFramePeriod, Profile, UsageType,
        VuiConfig,
    },
};
use std::{sync::Arc, time::Duration};

#[cfg(all(target_os = "linux", feature = "vaapi"))]
#[path = "vaapi.rs"]
pub mod vaapi;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoCodec {
    #[default]
    H264,
    Vp8,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoMode {
    #[default]
    Text,
    Motion,
}
impl VideoMode {
    pub fn max_size(self) -> (u32, u32) {
        match self {
            Self::Text => (3840, 2160),
            Self::Motion => (1920, 1080),
        }
    }
    pub fn fps(self) -> u32 {
        match self {
            Self::Text => 15,
            Self::Motion => 60,
        }
    }
}
#[derive(Clone, Debug)]
pub struct EncoderSettings {
    pub codec: VideoCodec,
    pub mode: VideoMode,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u32,
}
impl EncoderSettings {
    pub fn validate(&self) -> Result<()> {
        let (max_w, max_h) = self.mode.max_size();
        if self.width < 2
            || self.height < 2
            || !self.width.is_multiple_of(2)
            || !self.height.is_multiple_of(2)
            || self.width > max_w
            || self.height > max_h
            || self.fps == 0
            || self.fps > self.mode.fps()
            || self.bitrate < 32_000
            || self.bitrate > 50_000_000
        {
            return Err(VideoError::Invalid(
                "encoder dimensions, frame rate or bitrate out of range",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct EncodedVideoFrame {
    pub codec: VideoCodec,
    pub data: Arc<[u8]>,
    pub timestamp: Duration,
    pub keyframe: bool,
    pub width: u32,
    pub height: u32,
    pub rid: Option<String>,
}
pub trait VideoEncoder {
    fn encode(
        &mut self,
        frame: &I420Frame,
        timestamp: Duration,
    ) -> Result<Option<EncodedVideoFrame>>;
    fn request_keyframe(&mut self);
    fn set_bitrate(&mut self, bitrate: u32) -> Result<()>;
    fn backend(&self) -> &'static str;
}
fn encoder_error(e: impl std::fmt::Display) -> VideoError {
    VideoError::Encoder(e.to_string())
}

pub struct SoftwareH264 {
    inner: openh264::encoder::Encoder,
    settings: EncoderSettings,
    restart: bool,
}
impl SoftwareH264 {
    pub fn new(settings: EncoderSettings) -> Result<Self> {
        settings.validate()?;
        if settings.codec != VideoCodec::H264 {
            return Err(VideoError::Invalid("H264 encoder codec"));
        }
        let inner = Self::create(&settings)?;
        Ok(Self {
            inner,
            settings,
            restart: false,
        })
    }
    fn create(settings: &EncoderSettings) -> Result<openh264::encoder::Encoder> {
        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(settings.bitrate))
            .max_frame_rate(FrameRate::from_hz(settings.fps as f32))
            .skip_frames(true)
            .adaptive_quantization(settings.mode != VideoMode::Text)
            .background_detection(settings.mode != VideoMode::Text)
            .usage_type(if settings.mode == VideoMode::Text {
                UsageType::ScreenContentRealTime
            } else {
                UsageType::CameraVideoRealTime
            })
            .profile(Profile::Baseline)
            .num_threads(2)
            .vui(VuiConfig::bt709())
            .intra_frame_period(IntraFramePeriod::from_num_frames(settings.fps * 2));
        openh264::encoder::Encoder::with_api_config(OpenH264API::from_source(), config)
            .map_err(encoder_error)
    }
}
impl VideoEncoder for SoftwareH264 {
    fn encode(
        &mut self,
        frame: &I420Frame,
        timestamp: Duration,
    ) -> Result<Option<EncodedVideoFrame>> {
        if (frame.width, frame.height) != (self.settings.width, self.settings.height) {
            return Err(VideoError::Invalid("encoder frame dimensions changed"));
        }
        if self.restart {
            self.inner = Self::create(&self.settings)?;
            self.restart = false;
        }
        let bitstream = self
            .inner
            .encode_at(frame, Timestamp::from_millis(timestamp.as_millis() as u64))
            .map_err(encoder_error)?;
        if matches!(bitstream.frame_type(), FrameType::Skip | FrameType::Invalid) {
            return Ok(None);
        }
        let keyframe = bitstream.frame_type() == FrameType::IDR;
        Ok(Some(EncodedVideoFrame {
            codec: VideoCodec::H264,
            data: bitstream.to_vec().into(),
            timestamp,
            keyframe,
            width: frame.width,
            height: frame.height,
            rid: None,
        }))
    }
    fn request_keyframe(&mut self) {
        self.inner.force_intra_frame();
    }
    fn set_bitrate(&mut self, bitrate: u32) -> Result<()> {
        if self.settings.bitrate != bitrate {
            let mut next = self.settings.clone();
            next.bitrate = bitrate;
            next.validate()?;
            self.settings = next;
            self.restart = true;
        }
        Ok(())
    }
    fn backend(&self) -> &'static str {
        "openh264"
    }
}

#[cfg(feature = "vp8")]
#[path = "vp8.rs"]
mod vp8;
#[cfg(feature = "vp8")]
pub use vp8::SoftwareVp8;

/// Hardware errors on creation or during encoding fall back to independent
/// software IDRs, so viewers never receive deltas referencing the failed GPU.
pub fn create_encoder(
    settings: EncoderSettings,
    prefer_hardware: bool,
) -> Result<Box<dyn VideoEncoder>> {
    settings.validate()?;
    if settings.codec == VideoCodec::Vp8 {
        #[cfg(feature = "vp8")]
        {
            return Ok(Box::new(SoftwareVp8::new(settings)?));
        }
        #[cfg(not(feature = "vp8"))]
        {
            return Err(VideoError::Unsupported("enable the vp8 feature"));
        }
    }
    #[cfg(all(target_os = "linux", feature = "vaapi"))]
    if prefer_hardware {
        return Ok(Box::new(vaapi::FallbackEncoder::new(settings)?));
    }
    let _ = prefer_hardware;
    Ok(Box::new(SoftwareH264::new(settings)?))
}
