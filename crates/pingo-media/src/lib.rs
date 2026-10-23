//! Native 48 kHz audio engine. DSP runs on a worker, never inside device callbacks.
pub mod audio;
pub mod pipeline;
pub mod rtc;

pub const SAMPLE_RATE: u32 = 48_000;
pub const FRAME_SAMPLES: usize = 480;
pub type MonoFrame = [f32; FRAME_SAMPLES];
pub type StereoFrame = [[f32; 2]; FRAME_SAMPLES];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Opus: {0}")]
    Opus(#[from] opus::Error),
    #[error("audio device: {0}")]
    Device(String),
    #[error("invalid audio configuration: {0}")]
    Invalid(&'static str),
    #[error("audio processing: {0}")]
    Processing(String),
    #[error("RTC: {0}")]
    Rtc(String),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("signaling: {0}")]
    Json(#[from] serde_json::Error),
    #[error("WebSocket: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
}
pub type Result<T> = std::result::Result<T, Error>;
