//! Native client transport, pipelines and mixer through the full HTTP/SFU app.
use super::Server;
use pingo_media::{
    StereoFrame,
    audio::{codec::Profile, denoise::NoiseLevel, jitter::JitterStats, level::Gate},
    pipeline::AudioSettings,
    rtc::{Client, ClientConfig, Command, PlaybackFrame, RtcEvent},
};
use pingo_proto::DataMsg;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    task::{JoinHandle, LocalSet},
};

struct Native {
    commands: mpsc::Sender<Command>,
    events: broadcast::Receiver<RtcEvent>,
    playback: mpsc::Receiver<PlaybackFrame>,
    observed: Vec<RtcEvent>,
    task: JoinHandle<pingo_media::Result<()>>,
}

fn settings(profile: Profile) -> AudioSettings {
    AudioSettings {
        profile,
        gate: Gate::Open,
        noise_level: NoiseLevel::Off,
        agc: false,
        normalize: false,
        ..Default::default()
    }
}

impl Native {
    async fn join(server: &Server, auth: &str, profile: Profile) -> Self {
        let mut config = ClientConfig::new(
            format!("ws://{}/rtc", server.address),
            server.token(auth).await,
        );
        // Keep the production wildcard client binding, including route selection.
        config.settings = settings(profile);
        let client = Client::connect(config).await.unwrap();
        let events = client.events.subscribe();
        let (commands, rx) = mpsc::channel(32);
        let (output, playback) = mpsc::channel(1024);
        let task = tokio::task::spawn_local(client.run_with_playback(None, rx, Some(output)));
        let mut this = Self {
            commands,
            events,
            playback,
            observed: vec![],
            task,
        };
        this.wait(|e| matches!(e, RtcEvent::Connected)).await;
        this
    }

    fn remember(&mut self, event: RtcEvent) {
        assert!(
            !matches!(event, RtcEvent::Error { .. } | RtcEvent::DeviceError(_)),
            "{event:?}"
        );
        self.observed.push(event);
    }

    async fn wait(&mut self, predicate: impl Fn(&RtcEvent) -> bool) {
        if self.observed.iter().any(&predicate) {
            return;
        }
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                let event = self
                    .events
                    .recv()
                    .await
                    .expect("native client stopped or lagged");
                let found = predicate(&event);
                self.remember(event);
                if found {
                    break;
                }
            }
        })
        .await
        .expect("native WebRTC event timeout");
    }

    fn id(&self) -> String {
        self.observed
            .iter()
            .find_map(|e| match e {
                RtcEvent::Joined { participant_id, .. } => Some(participant_id.clone()),
                _ => None,
            })
            .unwrap()
    }

    async fn record(&mut self, duration: Duration) -> Vec<PlaybackFrame> {
        // Discard output from negotiation; subsequent frames carry their own clocks.
        while self.playback.try_recv().is_ok() {}
        let deadline = tokio::time::Instant::now() + duration;
        let mut frames = vec![];
        loop {
            tokio::select! {
                frame = self.playback.recv() => frames.push(frame.expect("playback stopped")),
                event = self.events.recv() => self.remember(event.expect("native events lagged")),
                _ = tokio::time::sleep_until(deadline) => break,
            }
        }
        frames
    }

    fn stats(&self) -> &HashMap<String, JitterStats> {
        self.observed
            .iter()
            .rev()
            .find_map(|e| match e {
                RtcEvent::PlayoutStats(stats) if !stats.is_empty() => Some(stats),
                _ => None,
            })
            .expect("playout stats")
    }

    async fn stop(self) {
        self.commands.send(Command::Leave).await.unwrap();
        self.task.await.unwrap().unwrap();
    }
}

fn signal(hz: f32, frame: usize, profile: Profile, marker: bool) -> StereoFrame {
    std::array::from_fn(|i| {
        let n = frame * 480 + i;
        let t = n as f32 / 48_000.0;
        // Stable fundamental plus syllabic, harmonic bursts.
        let burst = if frame % 37 < 12 {
            0.025 * (std::f32::consts::TAU * hz * 3.0 * t).sin()
        } else {
            0.0
        };
        let left = 0.12 * (std::f32::consts::TAU * hz * t).sin() + burst;
        let click = if marker && frame == 200 && (120..360).contains(&i) {
            0.8 * (std::f32::consts::PI * (i - 120) as f32 / 240.0).sin()
                * (std::f32::consts::TAU * 3000.0 * t).sin()
        } else {
            0.0
        };
        let right = if profile == Profile::Studio {
            // Independent channel makes mono/downmix bugs visible.
            0.12 * (std::f32::consts::TAU * 660.0 * t).sin()
        } else {
            left
        };
        [left + click, right + click]
    })
}

fn feed(
    commands: mpsc::Sender<Command>,
    hz: f32,
    profile: Profile,
    marker: bool,
) -> (JoinHandle<()>, oneshot::Receiver<Instant>) {
    let (tx, rx) = oneshot::channel();
    let task = tokio::task::spawn_local(async move {
        let mut marker_tx = Some(tx);
        let mut tick = tokio::time::interval(Duration::from_millis(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        for frame in 0..1000 {
            tick.tick().await;
            let at = Instant::now();
            commands
                .send(Command::Capture(Box::new(signal(
                    hz, frame, profile, marker,
                ))))
                .await
                .unwrap();
            if frame == 200 {
                // Capture commands represent completed 10 ms input buffers.
                let _ = marker_tx
                    .take()
                    .unwrap()
                    .send(at - Duration::from_millis(10) + Duration::from_micros(2500));
            }
        }
    });
    (task, rx)
}

fn amplitude(samples: &[f32], hz: f64) -> f64 {
    let (sin, cos) = samples
        .iter()
        .enumerate()
        .fold((0.0, 0.0), |(s, c), (i, x)| {
            let phase = std::f64::consts::TAU * hz * i as f64 / 48_000.0;
            (s + *x as f64 * phase.sin(), c + *x as f64 * phase.cos())
        });
    2.0 * sin.hypot(cos) / samples.len() as f64
}

async fn two_clients(profile: Profile) {
    let server = Server::start(true).await;
    let a = Native::join(&server, &server.ann_auth, profile).await;
    let a_id = a.id();
    let mut b = Native::join(&server, &server.bob_auth, profile).await;
    // Server-initiated subscription offers must complete before audio starts.
    b.wait(|e| matches!(e, RtcEvent::TrackPublished(t) if t.participant_id == a_id))
        .await;
    let (feeding, marker) = feed(a.commands.clone(), 440.0, profile, true);
    let frames = b.record(Duration::from_millis(3800)).await;
    let marker_at = marker.await.unwrap();
    let warm: Vec<_> = frames
        .iter()
        .filter(|f| f.at > marker_at + Duration::from_millis(500))
        .flat_map(|f| f.frame.iter().map(|s| s[0]))
        .collect();
    let rms = (warm.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / warm.len() as f64).sqrt();
    assert!(rms > 0.025, "{profile:?} no audible signal: {rms}");
    let dominant = (420i32..=460)
        .max_by(|a, b| amplitude(&warm, *a as f64).total_cmp(&amplitude(&warm, *b as f64)))
        .unwrap();
    // ±6 Hz allows the playout controller's bounded 1% pitch correction.
    assert!((dominant - 440).abs() <= 6, "dominant frequency {dominant}");
    let heard_marker = frames
        .iter()
        .filter(|f| f.at >= marker_at)
        .find_map(|f| {
            f.frame
                .iter()
                .position(|s| s[0].abs() > 0.28)
                .map(|sample| f.at + Duration::from_secs_f64(sample as f64 / 48_000.0))
        })
        .expect("marker click did not reach playback");
    let latency = heard_marker.duration_since(marker_at).as_secs_f64() * 1000.0;
    // 150 ms is the product's broadband budget; localhost should leave room
    // for capture/packetization, codec lookahead, playout and scheduler jitter.
    assert!(latency < 150.0, "localhost mouth-to-ear {latency:.2} ms");
    assert!(b.observed.iter().any(|e| matches!(e, RtcEvent::Data(DataMsg::Speaking { participant_id, level }) if participant_id == &a_id && *level > 0.0)), "RFC6464/Speaking not delivered");
    for event in &b.observed {
        if let RtcEvent::Negotiated {
            local_sdp,
            remote_sdp,
        } = event
        {
            for sdp in [local_sdp, remote_sdp] {
                assert!(
                    sdp.contains("opus/48000/2")
                        && sdp.contains("stereo=1")
                        && sdp.contains("useinbandfec=1")
                        && sdp.contains("maxaveragebitrate=256000"),
                    "Opus negotiation: {sdp}"
                );
            }
        }
    }
    if profile == Profile::Studio {
        let right: Vec<_> = frames
            .iter()
            .filter(|f| f.at > marker_at + Duration::from_millis(500))
            .flat_map(|f| f.frame.iter().map(|s| s[1]))
            .collect();
        assert!(band_amplitude(&warm, 440) > 0.06 && band_amplitude(&right, 660) > 0.06);
        assert!(
            band_amplitude(&warm, 660) < 0.015 && band_amplitude(&right, 440) < 0.015,
            "stereo channels collapsed"
        );
    }
    println!(
        "{profile:?}: RMS {rms:.3}, dominant {dominant} Hz, mouth-to-ear {latency:.2} ms; {:?}",
        b.stats()
    );
    let mut muted = settings(profile);
    muted.gate = Gate::Muted;
    a.commands
        .send(Command::Settings(Box::new(muted)))
        .await
        .unwrap();
    server
        .wait_participants(|rows| {
            rows.iter()
                .any(|r| r["user"] == server.ann.to_string() && r["muted"] == true)
        })
        .await;
    let muted = b.record(Duration::from_millis(700)).await;
    // Drain queued audio and Opus PLC before checking the quiet tail.
    let tail: Vec<_> = muted
        .iter()
        .rev()
        .take(15)
        .flat_map(|f| f.frame.iter().map(|s| s[0]))
        .collect();
    let tail_rms =
        (tail.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt();
    assert!(tail_rms < 0.005, "mute still audible: {tail_rms}");
    feeding.abort();
    let _ = feeding.await;
    a.stop().await;
    b.wait(|e| matches!(e, RtcEvent::ParticipantLeft(id) if id == &a_id))
        .await;
    server
        .wait_participants(|rows| rows.len() == 1 && rows[0]["user"] == server.bob.to_string())
        .await;
    b.stop().await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_voice_through_embedded_server() {
    LocalSet::new().run_until(two_clients(Profile::Voice)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_studio_through_embedded_server() {
    LocalSet::new()
        .run_until(two_clients(Profile::Studio))
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_three_party_mix_contains_both_publishers() {
    LocalSet::new()
        .run_until(async {
            let server = Server::start(true).await;
            let cara = server
                .t
                .create_user("cara@example.com", None)
                .await
                .unwrap();
            let invite = server
                .t
                .as_user(server.ann)
                .mutation::<crate::invites::create>((server.space, None, None))
                .await
                .unwrap();
            server
                .t
                .as_user(cara)
                .mutation::<crate::invites::accept>((invite,))
                .await
                .unwrap();
            let cara_auth = server
                .t
                .app()
                .auth()
                .sign_in_as(server.t.app(), cara)
                .await
                .unwrap()
                .access_token;
            let a = Native::join(&server, &server.ann_auth, Profile::Voice).await;
            let a_id = a.id();
            let mut b = Native::join(&server, &server.bob_auth, Profile::Voice).await;
            b.wait(|e| matches!(e, RtcEvent::TrackPublished(t) if t.participant_id == a_id))
                .await;
            let c = Native::join(&server, &cara_auth, Profile::Voice).await;
            let c_id = c.id();
            b.wait(|e| matches!(e, RtcEvent::TrackPublished(t) if t.participant_id == c_id))
                .await;
            server.wait_participants(|rows| rows.len() == 3).await;
            let (feed_a, _) = feed(a.commands.clone(), 440.0, Profile::Voice, false);
            let (feed_c, _) = feed(c.commands.clone(), 880.0, Profile::Voice, false);
            let frames = b.record(Duration::from_millis(2400)).await;
            let mix: Vec<_> = frames
                .iter()
                .skip(100)
                .flat_map(|f| f.frame.iter().map(|s| s[0]))
                .collect();
            let a_amp = band_amplitude(&mix, 440);
            let c_amp = band_amplitude(&mix, 880);
            assert!(
                a_amp > 0.04 && c_amp > 0.04,
                "missing publisher: 440 Hz={a_amp:.3}, 880 Hz={c_amp:.3}"
            );
            assert!(
                band_amplitude(&mix, 600) < 0.01,
                "broadband noise is not a successful mix"
            );
            println!(
                "Three-party mix: 440 Hz={a_amp:.3}, 880 Hz={c_amp:.3}; {:?}",
                b.stats()
            );
            assert_eq!(b.stats().len(), 2);
            feed_a.abort();
            feed_c.abort();
            let _ = feed_a.await;
            let _ = feed_c.await;
            a.stop().await;
            c.stop().await;
            b.stop().await;
            server.stop().await;
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_unpublish_republish_and_unmute_resume_audio() {
    LocalSet::new().run_until(async {
        let server = Server::start(true).await;
        let a = Native::join(&server, &server.ann_auth, Profile::Voice).await;
        let a_id = a.id();
        let mut b = Native::join(&server, &server.bob_auth, Profile::Voice).await;
        b.wait(|e| matches!(e, RtcEvent::TrackPublished(t) if t.participant_id == a_id)).await;
        let (feeding, _) = feed(a.commands.clone(), 440.0, Profile::Voice, false);
        let first = b.record(Duration::from_millis(800)).await;
        assert!(first.iter().any(|f| f.frame.iter().any(|s| s[0].abs() > 0.05)));
        a.commands.send(Command::PublishAudio(false)).await.unwrap();
        b.wait(|e| matches!(e, RtcEvent::TrackUnpublished { participant_id, .. } if participant_id == &a_id)).await;
        let quiet = b.record(Duration::from_millis(300)).await;
        assert!(quiet.iter().all(|f| f.frame.iter().all(|s| s[0] == 0.0)), "unpublish left a stale buffer");
        // Initial publication no longer satisfies the next wait.
        b.observed.clear();
        a.commands.send(Command::PublishAudio(true)).await.unwrap();
        b.wait(|e| matches!(e, RtcEvent::TrackPublished(t) if t.participant_id == a_id)).await;
        let resumed = b.record(Duration::from_millis(1500)).await;
        let samples: Vec<_> = resumed.iter().skip(80).flat_map(|f| f.frame.iter().map(|s| s[0])).collect();
        assert_resumed_tone(&samples, "republish");
        assert_eq!(b.stats().len(), 1, "old subscriber mid retained");
        let mut muted = settings(Profile::Voice);
        muted.gate = Gate::Muted;
        a.commands.send(Command::Settings(Box::new(muted))).await.unwrap();
        b.wait(|e| matches!(e, RtcEvent::ParticipantUpdated(p) if p.participant_id == a_id && p.muted)).await;
        let quiet = b.record(Duration::from_millis(300)).await;
        assert!(quiet.iter().all(|f| f.frame.iter().all(|s| s[0] == 0.0)));
        a.commands.send(Command::Settings(Box::new(settings(Profile::Voice)))).await.unwrap();
        b.wait(|e| matches!(e, RtcEvent::ParticipantUpdated(p) if p.participant_id == a_id && !p.muted)).await;
        let resumed = b.record(Duration::from_millis(1500)).await;
        let samples: Vec<_> = resumed.iter().skip(80).flat_map(|f| f.frame.iter().map(|s| s[0])).collect();
        assert_resumed_tone(&samples, "unmute");
        feeding.abort(); let _ = feeding.await;
        a.stop().await; b.stop().await; server.stop().await;
    }).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_connects_to_wildcard_sfu_udp_binding() {
    LocalSet::new()
        .run_until(async {
            let server = Server::start_udp(true, "0.0.0.0:0").await;
            let a = Native::join(&server, &server.ann_auth, Profile::Voice).await;
            a.stop().await;
            server.stop().await;
        })
        .await;
}

fn assert_resumed_tone(samples: &[f32], operation: &str) {
    let peak = band_amplitude(samples, 440);
    let rms =
        (samples.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / samples.len() as f64).sqrt();
    assert!(
        peak > 0.04 && rms > 0.03,
        "{operation}: peak {peak:.4}, RMS {rms:.4}"
    );
}

fn band_amplitude(samples: &[f32], hz: i32) -> f64 {
    // Adaptive playout changes both phase and pitch while correcting delay.
    // Use 100 ms spectral windows, searching ±2%, rather than cancelling a
    // moving tone against one phase reference across the entire recording.
    let tolerance = (hz as f64 * 0.02).ceil() as i32;
    let windows: Vec<_> = samples
        .as_chunks::<4800>()
        .0
        .iter()
        .map(|window| {
            ((hz - tolerance)..=(hz + tolerance))
                .step_by(2)
                .map(|frequency| amplitude(window, frequency as f64))
                .fold(0.0, f64::max)
        })
        .collect();
    assert!(
        !windows.is_empty(),
        "recording too short for spectral analysis"
    );
    windows.iter().sum::<f64>() / windows.len() as f64
}
