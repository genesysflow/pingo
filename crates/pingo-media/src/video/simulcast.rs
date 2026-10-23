use super::{
    Result, VideoError,
    capture::CapturedFrame,
    color::I420Frame,
    encode::{
        EncodedVideoFrame, EncoderSettings, VideoCodec, VideoEncoder, VideoMode, create_encoder,
    },
};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VideoSource {
    Screen,
    Camera,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    pub rid: String,
    pub width: u32,
    pub height: u32,
    pub bitrate: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoPublishConfig {
    pub source: VideoSource,
    pub mode: VideoMode,
    pub codec: VideoCodec,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u32,
    pub prefer_hardware: bool,
}
impl VideoPublishConfig {
    pub fn screen(width: u32, height: u32, mode: VideoMode) -> Self {
        Self {
            source: VideoSource::Screen,
            mode,
            codec: VideoCodec::H264,
            width,
            height,
            fps: mode.fps(),
            bitrate: if mode == VideoMode::Text {
                4_000_000
            } else {
                8_000_000
            },
            prefer_hardware: true,
        }
    }
    pub fn layers(&self) -> Result<Vec<Layer>> {
        EncoderSettings {
            codec: self.codec,
            mode: self.mode,
            width: self.width,
            height: self.height,
            fps: self.fps,
            bitrate: self.bitrate,
        }
        .validate()?;
        if self.mode == VideoMode::Text && self.fps < 5 {
            return Err(VideoError::Invalid("Text mode needs 5..15 fps"));
        }
        let scales: &[u32] = match self.source {
            VideoSource::Screen => &[2, 1],
            VideoSource::Camera => &[4, 2, 1],
        };
        Ok(scales
            .iter()
            .enumerate()
            .map(|(index, scale)| Layer {
                rid: match (self.source, index) {
                    (_, 0) => "l",
                    (VideoSource::Camera, 1) => "m",
                    _ => "h",
                }
                .into(),
                width: ((self.width / scale) & !1).max(2),
                height: ((self.height / scale) & !1).max(2),
                bitrate: (self.bitrate / scale.pow(2)).max(32_000),
            })
            .collect())
    }
}
struct LayerEncoder {
    layer: Layer,
    encoder: Box<dyn VideoEncoder>,
    target: u32,
}
pub struct SimulcastEncoder {
    pub config: VideoPublishConfig,
    layers: Vec<LayerEncoder>,
    last: Option<Duration>,
    last_budget: Option<u64>,
}
impl SimulcastEncoder {
    pub fn new(config: VideoPublishConfig) -> Result<Self> {
        let layers = config
            .layers()?
            .into_iter()
            .map(|layer| {
                let encoder = create_encoder(
                    EncoderSettings {
                        codec: config.codec,
                        mode: config.mode,
                        width: layer.width,
                        height: layer.height,
                        fps: config.fps,
                        bitrate: layer.bitrate,
                    },
                    config.prefer_hardware,
                )?;
                Ok(LayerEncoder {
                    target: layer.bitrate,
                    layer,
                    encoder,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            config,
            layers,
            last: None,
            last_budget: None,
        })
    }
    pub fn backends(&self) -> Vec<&'static str> {
        self.layers.iter().map(|l| l.encoder.backend()).collect()
    }
    pub fn request_keyframe(&mut self, rid: Option<&str>) {
        for layer in &mut self.layers {
            if rid.is_none_or(|rid| rid == layer.layer.rid) {
                layer.encoder.request_keyframe();
            }
        }
        // A PLI must be serviced even if no new damage has been reported.
        self.last = None;
    }
    pub fn set_bandwidth(&mut self, bits_per_second: u64) -> Result<()> {
        let budget = bits_per_second.saturating_sub(128_000).saturating_mul(8) / 10;
        if self
            .last_budget
            .is_some_and(|old| old.abs_diff(budget) < old / 5)
        {
            return Ok(());
        }
        let total = self
            .layers
            .iter()
            .map(|l| l.layer.bitrate as u64)
            .sum::<u64>();
        for layer in &mut self.layers {
            let target = (budget.saturating_mul(layer.layer.bitrate as u64) / total)
                .clamp(32_000, layer.layer.bitrate as u64) as u32;
            if target != layer.target {
                layer.encoder.set_bitrate(target)?;
                layer.target = target;
            }
        }
        self.last_budget = Some(budget);
        Ok(())
    }
    pub fn encode(&mut self, frame: &CapturedFrame) -> Result<Vec<EncodedVideoFrame>> {
        frame.validate()?;
        if self.last.is_some_and(|last| {
            (frame.timestamp.saturating_sub(last) + Duration::from_micros(500))
                < Duration::from_secs_f64(1.0 / self.config.fps as f64)
        }) {
            return Ok(Vec::new());
        }
        let source = I420Frame::from_capture(frame)?;
        let mut result = Vec::with_capacity(self.layers.len());
        for layer in &mut self.layers {
            let scaled = source.scale(layer.layer.width, layer.layer.height)?;
            if let Some(mut encoded) = layer.encoder.encode(&scaled, frame.timestamp)? {
                encoded.rid = Some(layer.layer.rid.clone());
                result.push(encoded);
            }
        }
        self.last = Some(frame.timestamp);
        Ok(result)
    }
}
