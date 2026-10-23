//! Capture, conversion and encoding live outside audio callbacks and RTC I/O.
pub mod capture;

#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    #[error("invalid video configuration: {0}")]
    Invalid(&'static str),
    #[error("capture: {0}")]
    Capture(String),
    #[error("encoder: {0}")]
    Encoder(String),
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
}
pub type Result<T> = std::result::Result<T, VideoError>;
