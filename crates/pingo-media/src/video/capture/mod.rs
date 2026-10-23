use super::{Result, VideoError};
use std::{future::Future, pin::Pin, time::Duration};

#[cfg(all(target_os = "linux", feature = "linux-capture"))]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod synthetic;
#[cfg(target_os = "windows")]
pub mod windows;

pub type CaptureFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorMode {
    Hidden,
    #[default]
    Embedded,
}
#[derive(Clone, Debug)]
pub struct CaptureOptions {
    /// Linux uses "monitor" or "window" to restrict the portal's own picker.
    pub source: String,
    pub cursor: CursorMode,
    pub restore_token: Option<String>,
    pub max_fps: u32,
}
impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            source: "monitor".into(),
            cursor: CursorMode::Embedded,
            restore_token: None,
            max_fps: 15,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Bgrx,
    Rgbx,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: PixelFormat,
    pub data: Vec<u8>,
    /// Monotonic time since this capture started; independent of the wall clock.
    pub timestamp: Duration,
    /// None means unknown/full damage; an empty list means unchanged content.
    pub damage: Option<Vec<DamageRect>>,
}
impl CapturedFrame {
    pub fn validate(&self) -> Result<()> {
        if self.width == 0 || self.height == 0 || self.width > 8192 || self.height > 8192 {
            return Err(VideoError::Invalid("capture dimensions must be 1..8192"));
        }
        if self.stride < self.width as usize * 4
            || self.data.len() < self.stride * self.height as usize
        {
            return Err(VideoError::Invalid("short packed pixel buffer"));
        }
        Ok(())
    }
}

/// Portal backends list picker categories, since the portal deliberately hides
/// monitor/window identities until the user grants access.
pub trait ScreenCapturer: Send {
    fn list_sources(&self) -> CaptureFuture<'_, Vec<Source>>;
    fn start(&mut self, options: CaptureOptions) -> CaptureFuture<'_, ()>;
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<CapturedFrame>>;
    fn stop(&mut self) -> CaptureFuture<'_, ()>;
    fn restore_token(&self) -> Option<&str> {
        None
    }
}
pub trait CameraCapturer: Send {
    fn list_sources(&self) -> CaptureFuture<'_, Vec<Source>>;
    fn start(&mut self, options: CaptureOptions) -> CaptureFuture<'_, ()>;
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<CapturedFrame>>;
    fn stop(&mut self) -> CaptureFuture<'_, ()>;
}
