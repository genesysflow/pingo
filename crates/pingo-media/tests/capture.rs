#![cfg(all(target_os = "linux", feature = "linux-capture"))]

use openh264::{decoder::Decoder, formats::YUVSource};
use pingo_media::video::{
    capture::{CaptureOptions, ScreenCapturer, linux::PortalScreen},
    color::I420Frame,
    encode::{EncoderSettings, SoftwareH264, VideoCodec, VideoEncoder, VideoMode},
};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires an interactive ScreenCast portal picker and PipeWire"]
async fn portal_capture_encodes_a_real_frame() {
    let mut capture = PortalScreen::default();
    let started = tokio::time::timeout(
        Duration::from_secs(45),
        capture.start(CaptureOptions::default()),
    )
    .await;
    if started.is_err() {
        capture.stop().await.unwrap();
    }
    started
        .expect("accept the portal picker within 45 seconds")
        .unwrap();
    let (mut capture, frame) = tokio::task::spawn_blocking(move || {
        let frame = capture.next_frame(Duration::from_secs(5));
        (capture, frame)
    })
    .await
    .unwrap();
    capture.stop().await.unwrap();
    let frame = frame.unwrap().expect("portal delivered no frame");
    eprintln!(
        "portal capture: {}x{} {:?}",
        frame.width, frame.height, frame.format
    );
    let yuv = I420Frame::from_capture(&frame)
        .unwrap()
        .scale(640, 360)
        .unwrap();
    let mut encoder = SoftwareH264::new(EncoderSettings {
        codec: VideoCodec::H264,
        mode: VideoMode::Text,
        width: 640,
        height: 360,
        fps: 15,
        bitrate: 2_000_000,
    })
    .unwrap();
    let packet = encoder.encode(&yuv, Duration::ZERO).unwrap().unwrap();
    let mut decoder = Decoder::new().unwrap();
    let decoded = decoder.decode(&packet.data).unwrap().unwrap();
    assert_eq!(decoded.dimensions(), (640, 360));
}
