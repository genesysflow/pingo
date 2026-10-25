//! Deterministic text-like and moving sources for hardware-free end-to-end tests.
use super::*;

pub struct SyntheticScreen {
    width: u32,
    height: u32,
    fps: u32,
    frame: u64,
    motion: bool,
    running: bool,
}
impl SyntheticScreen {
    pub fn new(width: u32, height: u32, motion: bool) -> Result<Self> {
        if width < 2 || height < 2 || width > 4096 || height > 2160 {
            return Err(VideoError::Invalid("synthetic size must fit 4096x2160"));
        }
        Ok(Self {
            width,
            height,
            fps: 15,
            frame: 0,
            motion,
            running: false,
        })
    }
    pub fn frame_at(&self, index: u64) -> CapturedFrame {
        let mut data = vec![0; self.width as usize * self.height as usize * 4];
        for y in 0..self.height {
            for x in 0..self.width {
                let shift = if self.motion { index as u32 * 4 } else { 0 };
                let glyph =
                    (x + shift) % 48 < 24 && y % 28 < 18 && ((x + shift) % 8 < 2 || y % 9 < 2);
                let moving = self.motion && (x + shift) % self.width < self.width / 4;
                let value = if glyph {
                    220
                } else if moving {
                    100
                } else {
                    28
                };
                let i = (y as usize * self.width as usize + x as usize) * 4;
                data[i..i + 4].copy_from_slice(&[value, value, value, 255]);
            }
        }
        CapturedFrame {
            width: self.width,
            height: self.height,
            stride: self.width as usize * 4,
            format: PixelFormat::Bgrx,
            data,
            timestamp: Duration::from_secs_f64(index as f64 / self.fps as f64),
            damage: None,
        }
    }
}
impl ScreenCapturer for SyntheticScreen {
    fn list_sources(&self) -> CaptureFuture<'_, Vec<Source>> {
        Box::pin(async {
            Ok(vec![Source {
                id: "synthetic".into(),
                name: "Synthetic screen".into(),
            }])
        })
    }
    fn start(&mut self, options: CaptureOptions) -> CaptureFuture<'_, ()> {
        Box::pin(async move {
            if !(1..=60).contains(&options.max_fps) {
                return Err(VideoError::Invalid("fps must be 1..60"));
            }
            self.fps = options.max_fps;
            self.frame = 0;
            self.running = true;
            Ok(())
        })
    }
    fn next_frame(&mut self, _: Duration) -> Result<Option<CapturedFrame>> {
        if !self.running {
            return Ok(None);
        }
        let frame = self.frame_at(self.frame);
        self.frame += 1;
        Ok(Some(frame))
    }
    fn stop(&mut self) -> CaptureFuture<'_, ()> {
        Box::pin(async {
            self.running = false;
            Ok(())
        })
    }
}
