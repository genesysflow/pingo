use openh264::{decoder::Decoder, formats::YUVSource};
use pingo_media::video::{
    capture::{CapturedFrame, PixelFormat, synthetic::SyntheticScreen},
    color::{I420Frame, psnr},
    encode::{EncoderSettings, SoftwareH264, VideoCodec, VideoEncoder, VideoMode},
    simulcast::{SimulcastEncoder, VideoPublishConfig, VideoSource},
};
use std::time::{Duration, Instant};

#[test]
fn bt709_conversion_handles_colors_padding_and_nv12() {
    let colors = [
        ([0, 0, 0, 255], 16, 128, 128),
        ([255, 255, 255, 255], 235, 128, 128),
        ([0, 0, 255, 255], 63, 102, 240),
        ([255, 0, 0, 255], 32, 240, 118),
        ([0, 255, 0, 255], 173, 42, 26),
    ];
    for (color, y, u, v) in colors {
        let mut data = vec![0; 24];
        data[..4].copy_from_slice(&color);
        data[4..8].copy_from_slice(&color);
        data[12..16].copy_from_slice(&color);
        data[16..20].copy_from_slice(&color);
        let frame = CapturedFrame {
            width: 2,
            height: 2,
            stride: 12,
            format: PixelFormat::Bgrx,
            data,
            timestamp: Duration::ZERO,
            damage: None,
        };
        let converted = I420Frame::from_capture(&frame).unwrap();
        for (actual, expected) in [
            (converted.y()[0], y),
            (converted.u()[0], u),
            (converted.v()[0], v),
        ] {
            assert!(
                (actual as i32 - expected).abs() <= 1,
                "{color:?}: {actual} != {expected}"
            );
        }
        let nv12 = converted.nv12();
        assert_eq!(&nv12[..4], converted.y());
        assert_eq!(&nv12[4..], &[converted.u()[0], converted.v()[0]]);
        let mut rgb = frame.clone();
        rgb.format = PixelFormat::Rgbx;
        for pixel in rgb.data.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        assert_eq!(
            I420Frame::from_capture(&rgb).unwrap().data(),
            converted.data()
        );
    }
    let mut odd = SyntheticScreen::new(161, 91, false).unwrap().frame_at(0);
    odd.data.pop();
    assert!(I420Frame::from_capture(&odd).is_err());
    odd.data.push(0);
    assert_eq!(
        I420Frame::from_capture(&odd).unwrap().dimensions(),
        (160, 90)
    );
}
fn encoder_settings(mode: VideoMode) -> EncoderSettings {
    EncoderSettings {
        codec: VideoCodec::H264,
        mode,
        width: 320,
        height: 180,
        fps: mode.fps(),
        bitrate: 2_000_000,
    }
}
#[test]
fn h264_roundtrip_text_motion_psnr_and_pli() {
    for mode in [VideoMode::Text, VideoMode::Motion] {
        let mut encoder = SoftwareH264::new(encoder_settings(mode)).unwrap();
        let mut decoder = Decoder::new().unwrap();
        let source = SyntheticScreen::new(320, 180, mode == VideoMode::Motion).unwrap();
        let start = Instant::now();
        let mut min = f64::INFINITY;
        let mut bytes = 0;
        for index in 0..30 {
            if index == 15 {
                encoder.request_keyframe();
            }
            if index == 20 {
                encoder.set_bitrate(1_000_000).unwrap();
            }
            let frame = I420Frame::from_capture(&source.frame_at(index)).unwrap();
            let encoded = encoder
                .encode(
                    &frame,
                    Duration::from_secs_f64(index as f64 / mode.fps() as f64),
                )
                .unwrap()
                .unwrap();
            if index == 0 || index == 15 || index == 20 {
                assert!(encoded.keyframe, "index {index}");
            }
            bytes += encoded.data.len();
            let decoded = decoder.decode(&encoded.data).unwrap().unwrap();
            assert_eq!(decoded.dimensions(), (320, 180));
            min = min.min(psnr(&frame, &decoded).unwrap());
        }
        eprintln!(
            "{mode:?}: minimum YUV PSNR {min:.2} dB, {:.2} ms/frame including conversion+decode, {bytes} bytes",
            start.elapsed().as_secs_f64() * 1000.0 / 30.0
        );
        assert!(min > 32.0, "{mode:?}: PSNR {min}");
    }
}
#[test]
fn simulcast_sizes_cadence_and_pli_on_requested_layer() {
    let mut config = VideoPublishConfig::screen(1920, 1080, VideoMode::Motion);
    let layers = config.layers().unwrap();
    assert_eq!(
        layers
            .iter()
            .map(|l| (&*l.rid, l.width, l.height))
            .collect::<Vec<_>>(),
        vec![("l", 960, 540), ("h", 1920, 1080)]
    );
    config.source = VideoSource::Camera;
    assert_eq!(
        config
            .layers()
            .unwrap()
            .iter()
            .map(|l| (&*l.rid, l.width, l.height))
            .collect::<Vec<_>>(),
        vec![("l", 480, 270), ("m", 960, 540), ("h", 1920, 1080)]
    );
    config = VideoPublishConfig::screen(320, 180, VideoMode::Text);
    config.prefer_hardware = false;
    let mut encoder = SimulcastEncoder::new(config).unwrap();
    let source = SyntheticScreen::new(320, 180, false).unwrap();
    let frame = source.frame_at(0);
    let first = encoder.encode(&frame).unwrap();
    assert_eq!(first.len(), 2);
    assert!(first.iter().all(|f| f.keyframe));
    assert!(encoder.encode(&frame).unwrap().is_empty());
    encoder.request_keyframe(Some("l"));
    let next = encoder.encode(&frame).unwrap();
    assert!(
        next.iter()
            .find(|f| f.rid.as_deref() == Some("l"))
            .unwrap()
            .keyframe
    );
    encoder.set_bandwidth(500_000).unwrap();
    let mut later = source.frame_at(2);
    later.timestamp = Duration::from_millis(200);
    assert_eq!(encoder.encode(&later).unwrap().len(), 2);
    encoder.set_bandwidth(u64::MAX).unwrap();
    encoder.set_bandwidth(0).unwrap();
}

#[cfg(feature = "vp8")]
#[test]
fn vp8_roundtrip_psnr_and_keyframe_on_request() {
    use ffmpeg_next::{Packet, codec, decoder, frame};
    use pingo_media::video::encode::SoftwareVp8;
    struct Decoded(frame::Video);
    impl YUVSource for Decoded {
        fn dimensions(&self) -> (usize, usize) {
            (self.0.width() as usize, self.0.height() as usize)
        }
        fn strides(&self) -> (usize, usize, usize) {
            (self.0.stride(0), self.0.stride(1), self.0.stride(2))
        }
        fn y(&self) -> &[u8] {
            self.0.data(0)
        }
        fn u(&self) -> &[u8] {
            self.0.data(1)
        }
        fn v(&self) -> &[u8] {
            self.0.data(2)
        }
    }
    for mode in [VideoMode::Text, VideoMode::Motion] {
        let mut settings = encoder_settings(mode);
        settings.codec = VideoCodec::Vp8;
        let mut encoder = SoftwareVp8::new(settings).unwrap();
        let source = SyntheticScreen::new(320, 180, mode == VideoMode::Motion).unwrap();
        let codec = decoder::find(codec::Id::VP8).unwrap();
        let mut decoder = codec::context::Context::new_with_codec(codec)
            .decoder()
            .video()
            .unwrap();
        let mut minimum = f64::INFINITY;
        for index in 0..30 {
            let frame = I420Frame::from_capture(&source.frame_at(index)).unwrap();
            if index == 15 {
                encoder.request_keyframe();
            }
            if index == 20 {
                encoder.set_bitrate(1_000_000).unwrap();
            }
            let encoded = encoder
                .encode(
                    &frame,
                    Duration::from_secs_f64(index as f64 / mode.fps() as f64),
                )
                .unwrap()
                .unwrap();
            assert_eq!(encoded.codec, VideoCodec::Vp8);
            if index == 0 || index == 15 || index == 20 {
                assert!(encoded.keyframe);
                assert_eq!(&encoded.data[3..6], &[0x9d, 0x01, 0x2a]);
            }
            decoder.send_packet(&Packet::copy(&encoded.data)).unwrap();
            let mut decoded = Decoded(frame::Video::empty());
            decoder.receive_frame(&mut decoded.0).unwrap();
            assert_eq!(decoded.dimensions(), (320, 180));
            minimum = minimum.min(psnr(&frame, &decoded).unwrap());
        }
        eprintln!("VP8 {mode:?}: minimum YUV PSNR {minimum:.2} dB");
        assert!(minimum > 32.0, "VP8 {mode:?}: PSNR {minimum}");
    }
}

#[cfg(all(target_os = "linux", feature = "vaapi"))]
#[test]
#[ignore = "requires a VAAPI render device"]
fn vaapi_real_encode_decode() {
    use pingo_media::video::encode::vaapi::VaapiH264;
    let mut encoder = VaapiH264::new(encoder_settings(VideoMode::Motion)).unwrap();
    let source = SyntheticScreen::new(320, 180, true).unwrap();
    let mut decoder = Decoder::new().unwrap();
    let mut decoded_count = 0;
    for index in 0..30 {
        let frame = I420Frame::from_capture(&source.frame_at(index)).unwrap();
        if index == 15 {
            encoder.request_keyframe();
        }
        if let Some(packet) = encoder
            .encode(&frame, Duration::from_millis(index * 20))
            .unwrap()
        {
            if index == 15 {
                assert!(packet.keyframe);
            }
            if let Some(decoded) = decoder.decode(&packet.data).unwrap() {
                assert_eq!(decoded.dimensions(), (320, 180));
                decoded_count += 1;
            }
        }
    }
    assert!(decoded_count >= 28);
    eprintln!("VAAPI frames decoded: {decoded_count}/30");
}
