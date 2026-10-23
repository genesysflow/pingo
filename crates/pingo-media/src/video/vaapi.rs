//! FFmpeg's VAAPI wrapper uploads NV12 into driver-owned frames. This path links
//! system FFmpeg/libva; no ffmpeg subprocess and no x264 encoder are used.
use super::*;
use ffmpeg::{Dictionary, Packet, codec, encoder, ffi, format::Pixel, frame, picture};
use ffmpeg_next as ffmpeg;
use std::{ffi::CString, ptr};

struct Buffer(*mut ffi::AVBufferRef);
impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            ffi::av_buffer_unref(&mut self.0);
        }
    }
}
fn check(code: i32) -> Result<()> {
    if code < 0 {
        Err(encoder_error(ffmpeg::Error::from(code)))
    } else {
        Ok(())
    }
}

pub struct VaapiH264 {
    // Encoder must drop before the frame pool/device references.
    encoder: encoder::Video,
    frames: Buffer,
    _device: Buffer,
    settings: EncoderSettings,
    keyframe: bool,
}
impl VaapiH264 {
    pub fn new(settings: EncoderSettings) -> Result<Self> {
        settings.validate()?;
        ffmpeg::init().map_err(encoder_error)?;
        let path =
            std::env::var("PINGO_VAAPI_DEVICE").unwrap_or_else(|_| "/dev/dri/renderD128".into());
        let path =
            CString::new(path).map_err(|_| VideoError::Invalid("VAAPI device contains NUL"))?;
        let mut device = Buffer(ptr::null_mut());
        // SAFETY: pointers are initialized/owned by FFmpeg, checked on every
        // allocation and unref'd by Buffer. No FFmpeg object crosses threads.
        unsafe {
            check(ffi::av_hwdevice_ctx_create(
                &mut device.0,
                ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                path.as_ptr(),
                ptr::null_mut(),
                0,
            ))?;
        }
        let frames = Buffer(unsafe { ffi::av_hwframe_ctx_alloc(device.0) });
        if frames.0.is_null() {
            return Err(VideoError::Encoder("VAAPI frame pool allocation".into()));
        }
        unsafe {
            let context = (*frames.0).data.cast::<ffi::AVHWFramesContext>();
            (*context).format = ffi::AVPixelFormat::AV_PIX_FMT_VAAPI;
            (*context).sw_format = ffi::AVPixelFormat::AV_PIX_FMT_NV12;
            (*context).width = settings.width as i32;
            (*context).height = settings.height as i32;
            (*context).initial_pool_size = 4;
            check(ffi::av_hwframe_ctx_init(frames.0))?;
        }
        let codec = encoder::find_by_name("h264_vaapi")
            .ok_or(VideoError::Unsupported("FFmpeg h264_vaapi encoder"))?;
        let mut context = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(encoder_error)?;
        context.set_width(settings.width);
        context.set_height(settings.height);
        context.set_format(Pixel::VAAPI);
        context.set_time_base((1, 90_000));
        context.set_frame_rate(Some((settings.fps as i32, 1)));
        context.set_bit_rate(settings.bitrate as usize);
        context.set_max_b_frames(0);
        context.set_gop(settings.fps * 2);
        unsafe {
            (*context.as_mut_ptr()).hw_frames_ctx = ffi::av_buffer_ref(frames.0);
            if (*context.as_mut_ptr()).hw_frames_ctx.is_null() {
                return Err(VideoError::Encoder("VAAPI frame pool reference".into()));
            }
        }
        let mut options = Dictionary::new();
        options.set("profile", "constrained_baseline");
        options.set("async_depth", "1");
        let encoder = context.open_with(options).map_err(encoder_error)?;
        Ok(Self {
            encoder,
            frames,
            _device: device,
            settings,
            keyframe: true,
        })
    }
}
impl VideoEncoder for VaapiH264 {
    fn encode(
        &mut self,
        input: &I420Frame,
        timestamp: Duration,
    ) -> Result<Option<EncodedVideoFrame>> {
        if (input.width, input.height) != (self.settings.width, self.settings.height) {
            return Err(VideoError::Invalid("VAAPI frame dimensions changed"));
        }
        let nv12 = input.nv12();
        let mut software = frame::Video::new(Pixel::NV12, input.width, input.height);
        let ysize = input.width as usize * input.height as usize;
        for (plane, bytes, rows) in [
            (0, &nv12[..ysize], input.height),
            (1, &nv12[ysize..], input.height / 2),
        ] {
            let stride = software.stride(plane);
            for row in 0..rows as usize {
                software.data_mut(plane)[row * stride..row * stride + input.width as usize]
                    .copy_from_slice(
                        &bytes[row * input.width as usize..(row + 1) * input.width as usize],
                    );
            }
        }
        let mut hardware = frame::Video::empty();
        unsafe {
            check(ffi::av_hwframe_get_buffer(
                self.frames.0,
                hardware.as_mut_ptr(),
                0,
            ))?;
            check(ffi::av_hwframe_transfer_data(
                hardware.as_mut_ptr(),
                software.as_ptr(),
                0,
            ))?;
        }
        hardware.set_pts(Some((timestamp.as_secs_f64() * 90_000.0) as i64));
        if self.keyframe {
            hardware.set_kind(picture::Type::I);
            self.keyframe = false;
        }
        self.encoder.send_frame(&hardware).map_err(encoder_error)?;
        let mut packet = Packet::empty();
        match self.encoder.receive_packet(&mut packet) {
            Ok(()) => Ok(Some(EncodedVideoFrame {
                codec: VideoCodec::H264,
                data: Arc::from(packet.data().unwrap_or(&[])),
                timestamp: Duration::from_secs_f64(
                    packet.pts().unwrap_or(0).max(0) as f64 / 90_000.0,
                ),
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
        let mut settings = self.settings.clone();
        settings.bitrate = bitrate;
        *self = Self::new(settings)?;
        Ok(())
    }
    fn backend(&self) -> &'static str {
        "vaapi-h264"
    }
}

pub struct FallbackEncoder {
    hardware: Option<VaapiH264>,
    software: SoftwareH264,
}
impl FallbackEncoder {
    pub fn new(settings: EncoderSettings) -> Result<Self> {
        let hardware = match VaapiH264::new(settings.clone()) {
            Ok(hw) => Some(hw),
            Err(error) => {
                tracing::info!(%error,"VAAPI unavailable; using OpenH264");
                None
            }
        };
        Ok(Self {
            hardware,
            software: SoftwareH264::new(settings)?,
        })
    }
}
impl VideoEncoder for FallbackEncoder {
    fn encode(
        &mut self,
        frame: &I420Frame,
        timestamp: Duration,
    ) -> Result<Option<EncodedVideoFrame>> {
        if let Some(hardware) = &mut self.hardware {
            match hardware.encode(frame, timestamp) {
                Ok(encoded) => return Ok(encoded),
                Err(error) => {
                    tracing::warn!(%error,"VAAPI failed; switching to OpenH264");
                    self.hardware = None;
                    self.software.request_keyframe();
                }
            }
        }
        self.software.encode(frame, timestamp)
    }
    fn request_keyframe(&mut self) {
        if let Some(hw) = &mut self.hardware {
            hw.request_keyframe();
        }
        self.software.request_keyframe();
    }
    fn set_bitrate(&mut self, bitrate: u32) -> Result<()> {
        self.software.set_bitrate(bitrate)?;
        if let Some(hw) = &mut self.hardware
            && let Err(error) = hw.set_bitrate(bitrate)
        {
            tracing::warn!(%error,"VAAPI reconfiguration failed; switching to OpenH264");
            self.hardware = None;
        }
        Ok(())
    }
    fn backend(&self) -> &'static str {
        if self.hardware.is_some() {
            "vaapi-h264"
        } else {
            "openh264"
        }
    }
}
