//! libvpx through the system FFmpeg wrapper. The published vpx-encode crate's
//! generated bindings do not build with the installed libvpx/Clang combination.
use super::*;
use ffmpeg::{Dictionary, Packet, codec, encoder, format::Pixel, frame, picture};
use ffmpeg_next as ffmpeg;
use openh264::formats::YUVSource;

pub struct SoftwareVp8 {
    inner: encoder::Video,
    settings: EncoderSettings,
    keyframe: bool,
}
impl SoftwareVp8 {
    pub fn new(settings: EncoderSettings) -> Result<Self> {
        settings.validate()?;
        if settings.codec != VideoCodec::Vp8 {
            return Err(VideoError::Invalid("VP8 encoder codec"));
        }
        ffmpeg::init().map_err(encoder_error)?;
        let codec = encoder::find_by_name("libvpx")
            .ok_or(VideoError::Unsupported("FFmpeg libvpx encoder"))?;
        let mut context = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(encoder_error)?;
        context.set_width(settings.width);
        context.set_height(settings.height);
        context.set_format(Pixel::YUV420P);
        context.set_time_base((1, 90_000));
        context.set_frame_rate(Some((settings.fps as i32, 1)));
        context.set_bit_rate(settings.bitrate as usize);
        context.set_max_b_frames(0);
        context.set_gop(settings.fps * 2);
        context.set_threading(codec::threading::Config {
            count: 2,
            ..Default::default()
        });
        let mut options = Dictionary::new();
        options.set("deadline", "realtime");
        options.set("cpu-used", "4");
        options.set("lag-in-frames", "0");
        options.set("auto-alt-ref", "0");
        options.set("error-resilient", "1");
        let inner = context.open_with(options).map_err(encoder_error)?;
        Ok(Self {
            inner,
            settings,
            keyframe: true,
        })
    }
}
impl VideoEncoder for SoftwareVp8 {
    fn encode(
        &mut self,
        input: &I420Frame,
        timestamp: Duration,
    ) -> Result<Option<EncodedVideoFrame>> {
        if (input.width, input.height) != (self.settings.width, self.settings.height) {
            return Err(VideoError::Invalid("VP8 frame dimensions changed"));
        }
        let mut frame = frame::Video::new(Pixel::YUV420P, input.width, input.height);
        for (plane, bytes, w, h) in [
            (0, input.y(), input.width, input.height),
            (1, input.u(), input.width / 2, input.height / 2),
            (2, input.v(), input.width / 2, input.height / 2),
        ] {
            let stride = frame.stride(plane);
            for row in 0..h as usize {
                frame.data_mut(plane)[row * stride..row * stride + w as usize]
                    .copy_from_slice(&bytes[row * w as usize..(row + 1) * w as usize]);
            }
        }
        frame.set_pts(Some((timestamp.as_secs_f64() * 90_000.0) as i64));
        if self.keyframe {
            frame.set_kind(picture::Type::I);
            self.keyframe = false;
        }
        self.inner.send_frame(&frame).map_err(encoder_error)?;
        let mut packet = Packet::empty();
        match self.inner.receive_packet(&mut packet) {
            Ok(()) => Ok(Some(EncodedVideoFrame {
                codec: VideoCodec::Vp8,
                data: Arc::from(packet.data().unwrap_or(&[])),
                timestamp,
                keyframe: packet.is_key(),
                width: input.width,
                height: input.height,
                rid: None,
            })),
            Err(ffmpeg::Error::Other { errno: 11 }) => Ok(None),
            Err(e) => Err(encoder_error(e)),
        }
    }
    fn request_keyframe(&mut self) {
        self.keyframe = true;
    }
    fn set_bitrate(&mut self, bitrate: u32) -> Result<()> {
        if self.settings.bitrate != bitrate {
            let mut next = self.settings.clone();
            next.bitrate = bitrate;
            *self = Self::new(next)?;
        }
        Ok(())
    }
    fn backend(&self) -> &'static str {
        "libvpx"
    }
}
