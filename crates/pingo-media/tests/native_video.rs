//! Real in-process SFU and RxFerrix backend, using the production Client.
use openh264::{decoder::Decoder, formats::YUVSource};
use pingo_media::{
    pipeline::AudioSettings,
    rtc::{Client, ClientConfig, Command, PlaybackFrame, RtcEvent},
    video::{
        capture::{CaptureOptions, ScreenCapturer, synthetic::SyntheticScreen},
        color::{I420Frame, psnr},
        encode::{EncodedVideoFrame, VideoMode},
        simulcast::{VideoPublishConfig, VideoSource},
    },
};
use pingo_proto::{MediaClaims, TrackKind, token};
use pingo_sfu::{Sfu, SfuConfig, VoiceHooks};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{broadcast, mpsc};

struct Hooks;
#[async_trait::async_trait]
impl VoiceHooks for Hooks {
    async fn on_state(&self, _: &str, _: &str, _: bool, _: bool, _: bool, _: bool) {}
    async fn on_leave(&self, _: &str, _: &str) {}
}
struct Native {
    commands: mpsc::Sender<Command>,
    events: broadcast::Receiver<RtcEvent>,
    task: tokio::task::JoinHandle<pingo_media::Result<()>>,
    playback: mpsc::Receiver<PlaybackFrame>,
}
impl Native {
    async fn join(url: &str, token: String) -> Self {
        let mut config = ClientConfig::new(url.into(), token);
        config.settings = AudioSettings::default();
        if let Ok(url) = std::env::var("PINGO_TEST_TURN_URL") {
            let secret = std::env::var("PINGO_TEST_TURN_SECRET").unwrap();
            let expiry = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600;
            let username = format!("{expiry}:native-video");
            let credential = pingo_proto::ice::turn_password(&username, secret.as_bytes());
            config.ice_servers = vec![pingo_proto::ice::IceServer {
                urls: vec![url],
                username,
                credential,
            }];
            config.relay_only = true;
        }
        let relay_only = config.relay_only;
        let client = Client::connect(config).await.unwrap();
        let events = client.events.subscribe();
        let (commands, rx) = mpsc::channel(32);
        let (playback_tx, playback) = mpsc::channel(64);
        let task = tokio::task::spawn_local(client.run_with_playback(None, rx, Some(playback_tx)));
        let mut native = Self {
            commands,
            events,
            task,
            playback,
        };
        native
            .wait(|event| {
                if relay_only && let RtcEvent::Negotiated { local_sdp, .. } = event {
                    assert!(local_sdp.contains(" typ relay"));
                    assert!(!local_sdp.contains(" typ host"));
                }
                matches!(event, RtcEvent::Connected)
            })
            .await;
        native
    }
    async fn wait(&mut self, predicate: impl Fn(&RtcEvent) -> bool) -> RtcEvent {
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                let event = self.events.recv().await.unwrap();
                if predicate(&event) {
                    return event;
                }
                assert!(
                    !matches!(event, RtcEvent::Error { .. } | RtcEvent::DeviceError(_)),
                    "{event:?}"
                );
            }
        })
        .await
        .expect("native event timeout")
    }
    async fn stop(self) {
        self.commands.send(Command::Leave).await.unwrap();
        self.task.await.unwrap().unwrap();
    }
}
fn admission(user: &str) -> String {
    token::sign(
        &MediaClaims {
            room: "native-video".into(),
            user: user.into(),
            name: user.into(),
            color: "#abcd00".into(),
            can_share: true,
            can_speak: true,
            can_video: true,
            exp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as f64
                + 60_000.0,
        },
        b"native video integration",
    )
}
async fn exercise(url: &str, a_token: String, b_token: String, mode: VideoMode) {
    let a = Native::join(url, a_token).await;
    let mut b = Native::join(url, b_token).await;
    a.commands.send(Command::PublishAudio(false)).await.unwrap();
    let mut config = VideoPublishConfig::screen(192, 108, mode);
    config.prefer_hardware = false;
    // The 60 fps low layer also needs a quality budget. Congestion/bitrate
    // adaptation is covered independently; this fixture measures decode quality.
    config.bitrate = 2_000_000;
    // Protocol/cadence fixture stays small so debug conversion is not a
    // throughput benchmark. The release example measures 1080p/4K separately.
    a.commands
        .send(Command::PublishVideo {
            source: VideoSource::Screen,
            config: Some(config.clone()),
        })
        .await
        .unwrap();
    b.wait(|e| matches!(e,RtcEvent::TrackPublished(t) if t.kind==TrackKind::Screen))
        .await;
    let feed = a.commands.clone();
    let feeding = tokio::task::spawn_local(async move {
        let mut source = SyntheticScreen::new(192, 108, mode == VideoMode::Motion).unwrap();
        source
            .start(CaptureOptions {
                max_fps: mode.fps(),
                ..Default::default()
            })
            .await
            .unwrap();
        let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / mode.fps() as f64));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        for _ in 0..mode.fps() * 4 {
            tick.tick().await;
            let frame = source.next_frame(Duration::ZERO).unwrap().unwrap();
            feed.send(Command::VideoFrame {
                source: VideoSource::Screen,
                frame: Box::new(frame),
            })
            .await
            .unwrap();
        }
    });
    // Keep image generation, decoding and PSNR off the RTC executor. A debug
    // PSNR scan can otherwise stall the 10 ms audio clock for hundreds of ms.
    let mut frames = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(4200);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            event = b.events.recv() => match event.unwrap() {
                RtcEvent::VideoFrame { frame, .. } => frames.push(frame),
                RtcEvent::Error { code, message } => panic!("{code}: {message}"),
                RtcEvent::DeviceError(error) => panic!("{error}"),
                _ => {}
            }
        }
    }
    feeding.await.unwrap();
    tokio::task::spawn_blocking(move || verify_frames(frames, mode))
        .await
        .unwrap();
    // Renegotiated stop must be visible remotely.
    a.commands
        .send(Command::PublishVideo {
            source: VideoSource::Screen,
            config: None,
        })
        .await
        .unwrap();
    b.wait(|e| matches!(e, RtcEvent::TrackUnpublished { .. }))
        .await;
    a.stop().await;
    b.stop().await;
}
fn verify_frames(frames: Vec<EncodedVideoFrame>, mode: VideoMode) {
    let mut decoder = Decoder::new().unwrap();
    let mut timestamps = Vec::new();
    let mut sizes = Vec::new();
    let mut minimum = f64::INFINITY;
    let mut samples = 0;
    let source = SyntheticScreen::new(192, 108, mode == VideoMode::Motion).unwrap();
    for frame in frames {
        if let Some(decoded) = decoder.decode(&frame.data).unwrap() {
            let size = decoded.dimensions();
            assert!(matches!(size, (192, 108) | (96, 54)), "{size:?}");
            sizes.push(size);
            timestamps.push(frame.timestamp.as_secs_f64());
            if timestamps.len() % 15 == 1 {
                let mut best = f64::NEG_INFINITY;
                let guess = ((frame.timestamp.as_secs_f64() - 0.1).max(0.0) * mode.fps() as f64)
                    .round() as u64;
                let candidates = if mode == VideoMode::Motion {
                    guess.saturating_sub(4)..guess + 5
                } else {
                    0..1
                };
                for index in candidates {
                    let reference = I420Frame::from_capture(&source.frame_at(index))
                        .unwrap()
                        .scale(size.0 as u32, size.1 as u32)
                        .unwrap();
                    best = best.max(psnr(&reference, &decoded).unwrap());
                }
                minimum = minimum.min(best);
                samples += 1;
            }
        }
    }
    assert!(
        timestamps.len() >= if mode == VideoMode::Text { 35 } else { 90 },
        "only {} {mode:?} decoded frames",
        timestamps.len()
    );
    let mut intervals: Vec<_> = timestamps
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect();
    intervals.sort_by(f64::total_cmp);
    let fps = 1.0 / intervals[intervals.len() / 2];
    assert!(
        (fps - mode.fps() as f64).abs() < mode.fps() as f64 * 0.1,
        "{mode:?} {fps} fps"
    );
    assert!(minimum > 30.0, "{mode:?} PSNR {minimum}");
    assert!(samples >= 2);
    eprintln!(
        "native {mode:?}: {} decoded frames, median {fps:.2} fps, min sampled PSNR {minimum:.2} dB, resolutions {:?}",
        timestamps.len(),
        sizes
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
    );
}
async fn in_process(mode: VideoMode) {
    let (sfu, http, url) = server().await;
    exercise(&url, admission("publisher"), admission("viewer"), mode).await;
    sfu.shutdown().await;
    http.abort();
}
async fn server() -> (Sfu, tokio::task::JoinHandle<()>, String) {
    let sfu = Sfu::start(
        SfuConfig {
            udp_bind: "127.0.0.1:0".parse().unwrap(),
            media_secret: b"native video integration".to_vec(),
            ..Default::default()
        },
        Arc::new(Hooks),
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/rtc", listener.local_addr().unwrap());
    let router: axum::Router = sfu.router();
    let http = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (sfu, http, url)
}
#[tokio::test]
async fn native_screen_text_through_in_process_sfu() {
    tokio::task::LocalSet::new()
        .run_until(in_process(VideoMode::Text))
        .await;
}
#[tokio::test]
async fn native_screen_motion_through_in_process_sfu() {
    tokio::task::LocalSet::new()
        .run_until(in_process(VideoMode::Motion))
        .await;
}

#[tokio::test]
async fn failed_video_start_keeps_the_call_connected() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (sfu, http, url) = server().await;
            let mut a = Native::join(&url, admission("publisher")).await;
            let mut b = Native::join(&url, admission("viewer")).await;
            let mut config = VideoPublishConfig::screen(320, 180, VideoMode::Text);
            config.prefer_hardware = false;
            #[cfg(not(feature = "vp8"))]
            {
                config.codec = pingo_media::video::encode::VideoCodec::Vp8;
            }
            #[cfg(feature = "vp8")]
            {
                config.bitrate = 1;
            }
            a.commands
                .send(Command::PublishVideo {
                    source: VideoSource::Screen,
                    config: Some(config),
                })
                .await
                .unwrap();
            a.wait(|e| matches!(e, RtcEvent::DeviceError(_))).await;
            // ICE Connected precedes SCTP channel readiness. Use authoritative
            // signaling state to prove that a failed encoder did not kill the call.
            a.commands
                .send(Command::Settings(Box::new(AudioSettings {
                    gate: pingo_media::audio::level::Gate::Muted,
                    ..Default::default()
                })))
                .await
                .unwrap();
            b.wait(|e| matches!(e, RtcEvent::ParticipantUpdated(p) if p.muted))
                .await;
            // Stop also wins over any encoder startup still in flight.
            a.commands
                .send(Command::PublishVideo {
                    source: VideoSource::Screen,
                    config: None,
                })
                .await
                .unwrap();
            a.stop().await;
            b.stop().await;
            sfu.shutdown().await;
            http.abort();
        })
        .await;
}

#[tokio::test]
async fn native_camera_and_stereo_screen_audio_have_independent_tracks() {
    tokio::task::LocalSet::new().run_until(async {
        let (sfu, http, url) = server().await;
        let a = Native::join(&url, admission("publisher")).await;
        let mut b = Native::join(&url, admission("viewer")).await;
        a.commands.send(Command::PublishAudio(false)).await.unwrap();
        let mut config = VideoPublishConfig::screen(192, 108, VideoMode::Motion);
        config.source = VideoSource::Camera;
        config.fps = 25;
        config.bitrate = 2_000_000;
        config.prefer_hardware = false;
        a.commands.send(Command::PublishVideo { source: VideoSource::Camera, config: Some(config) }).await.unwrap();
        a.commands.send(Command::PublishScreenAudio(true)).await.unwrap();
        let mut camera = None;
        let mut audio = None;
        while camera.is_none() || audio.is_none() {
            if let RtcEvent::TrackPublished(track) = b.wait(|e| matches!(e, RtcEvent::TrackPublished(_))).await {
                match track.kind {
                    TrackKind::Camera => camera = Some(track.mid),
                    TrackKind::ScreenAudio => audio = Some(track.mid),
                    _ => {}
                }
            }
        }
        let feed = a.commands.clone();
        let feeding = tokio::task::spawn_local(async move {
            let mut screen = SyntheticScreen::new(192, 108, true).unwrap();
            screen.start(CaptureOptions { max_fps: 25, ..Default::default() }).await.unwrap();
            let mut tick = tokio::time::interval(Duration::from_millis(10));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            for quantum in 0..200 {
                tick.tick().await;
                if quantum % 4 == 0 {
                    feed.send(Command::VideoFrame { source: VideoSource::Camera,
                        frame: Box::new(screen.next_frame(Duration::ZERO).unwrap().unwrap()) }).await.unwrap();
                }
                let frame = Box::new(std::array::from_fn(|sample| {
                    let time = (quantum * pingo_media::FRAME_SAMPLES + sample) as f32 / 48_000.0;
                    [0.15 * (time * 440.0 * std::f32::consts::TAU).sin(),
                     0.15 * (time * 880.0 * std::f32::consts::TAU).sin()]
                }));
                feed.send(Command::CaptureScreenAudio(frame)).await.unwrap();
            }
        });
        let mut decoder = Decoder::new().unwrap();
        let mut decoded_frames = 0;
        let mut pcm = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(2400);
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => break,
                frame = b.playback.recv() => pcm.extend_from_slice(frame.unwrap().frame.as_slice()),
                event = b.events.recv() => match event.unwrap() {
                    RtcEvent::VideoFrame { track, frame } => {
                        assert_eq!(track.kind, TrackKind::Camera);
                        if let Some(decoded) = decoder.decode(&frame.data).unwrap() {
                            assert!(matches!(decoded.dimensions(), (48, 26) | (96, 54) | (192, 108)));
                            decoded_frames += 1;
                        }
                    }
                    RtcEvent::DeviceError(error) => panic!("{error}"),
                    RtcEvent::Error { code, message } => panic!("{code}: {message}"),
                    _ => {}
                }
            }
        }
        feeding.await.unwrap();
        assert!(decoded_frames >= 30, "only {decoded_frames} camera frames");
        let pcm = &pcm[pcm.len().saturating_sub(48_000)..];
        assert!(pcm.len() >= 48_000);
        let amplitude = |channel: usize, hz: f64| {
            let (mut re, mut im) = (0.0, 0.0);
            for (i, frame) in pcm.iter().enumerate() {
                let phase = i as f64 * hz * std::f64::consts::TAU / 48_000.0;
                re += frame[channel] as f64 * phase.cos();
                im += frame[channel] as f64 * phase.sin();
            }
            2.0 * re.hypot(im) / pcm.len() as f64
        };
        let left = amplitude(0, 440.0);
        let right = amplitude(1, 880.0);
        assert!(left > 0.02 && right > 0.02, "screen stereo amplitudes {left}/{right}");
        assert!(amplitude(0, 880.0) < left * 0.1);
        assert!(amplitude(1, 440.0) < right * 0.1);
        eprintln!("native camera: {decoded_frames} frames; ScreenAudio stereo amplitudes {left:.3}/{right:.3}");
        a.commands.send(Command::PublishVideo { source: VideoSource::Camera, config: None }).await.unwrap();
        a.commands.send(Command::PublishScreenAudio(false)).await.unwrap();
        while camera.is_some() || audio.is_some() {
            if let RtcEvent::TrackUnpublished { mid, .. } = b.wait(|e| matches!(e, RtcEvent::TrackUnpublished { .. })).await {
                if camera.as_ref() == Some(&mid) { camera = None; }
                if audio.as_ref() == Some(&mid) { audio = None; }
            }
        }
        a.stop().await;
        b.stop().await;
        sfu.shutdown().await;
        http.abort();
    }).await;
}

#[tokio::test]
async fn native_video_full_server_text_and_motion() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let server = pingo_media_test_server::Server::start().await;
            for mode in [VideoMode::Text, VideoMode::Motion] {
                let [a, b] = server.tokens().await;
                exercise(&server.url, a, b, mode).await;
                server.assert_empty().await;
            }
            server.stop().await;
        })
        .await;
}
