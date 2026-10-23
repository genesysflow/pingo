//! In-process SFU and optional full RxFerrix executable, using production Client.
use openh264::{decoder::Decoder, formats::YUVSource};
use pingo_media::{
    pipeline::AudioSettings,
    rtc::{Client, ClientConfig, Command, RtcEvent},
    video::{
        capture::{CaptureOptions, ScreenCapturer, synthetic::SyntheticScreen},
        color::{I420Frame, psnr},
        encode::VideoMode,
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
}
impl Native {
    async fn join(url: &str, token: String) -> Self {
        let client = Client::connect(ClientConfig {
            settings: AudioSettings::default(),
            ..ClientConfig::new(url.into(), token)
        })
        .await
        .unwrap();
        let events = client.events.subscribe();
        let (commands, rx) = mpsc::channel(32);
        let task = tokio::task::spawn_local(client.run(None, rx));
        let mut native = Self {
            commands,
            events,
            task,
        };
        native
            .wait(|event| matches!(event, RtcEvent::Connected))
            .await;
        native
    }
    async fn wait(&mut self, predicate: impl Fn(&RtcEvent) -> bool) -> RtcEvent {
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                let event = self.events.recv().await.unwrap();
                assert!(
                    !matches!(event, RtcEvent::Error { .. } | RtcEvent::DeviceError(_)),
                    "{event:?}"
                );
                if predicate(&event) {
                    return event;
                }
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
    let mut config = VideoPublishConfig::screen(320, 180, mode);
    config.prefer_hardware = false;
    config.bitrate = 1_000_000;
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
        let mut source = SyntheticScreen::new(320, 180, mode == VideoMode::Motion).unwrap();
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
    let mut decoder = Decoder::new().unwrap();
    let mut timestamps = Vec::new();
    let mut sizes = Vec::new();
    let mut minimum = f64::INFINITY;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut source = SyntheticScreen::new(320, 180, mode == VideoMode::Motion).unwrap();
    source
        .start(CaptureOptions {
            max_fps: mode.fps(),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut samples = 0;
    loop {
        tokio::select! {
            _=tokio::time::sleep_until(deadline)=>break,
            event=b.events.recv()=>match event.unwrap() {
                RtcEvent::VideoFrame { frame,.. }=>{
                    if let Some(decoded)=decoder.decode(&frame.data).unwrap() {
                        let size=decoded.dimensions();assert!(matches!(size,(320,180)|(160,90)),"{size:?}");
                        sizes.push(size);timestamps.push(frame.timestamp.as_secs_f64());
                        // Compare independent decoded samples against the deterministic
                        // generator around the source RTP clock (with a bounded
                        // allowance for SFU layer-switch clock adjustment).
                        if timestamps.len()%15==1 {
                            let mut best=f64::NEG_INFINITY;
                            let guess=((frame.timestamp.as_secs_f64()-0.1).max(0.0)*mode.fps() as f64).round() as u64;
                            let candidates=if mode==VideoMode::Motion { guess.saturating_sub(4)..guess+5 } else { 0..1 };
                            for index in candidates {
                                let reference=I420Frame::from_capture(&source.frame_at(index)).unwrap().scale(size.0 as u32,size.1 as u32).unwrap();
                                best=best.max(psnr(&reference,&decoded).unwrap());
                            }
                            minimum=minimum.min(best);samples+=1;
                        }
                    }
                }
                RtcEvent::Error { code,message }=>panic!("{code}: {message}"),
                RtcEvent::DeviceError(error)=>panic!("{error}"),
                _=>{}
            }
        }
    }
    feeding.await.unwrap();
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
async fn in_process(mode: VideoMode) {
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
    exercise(&url, admission("publisher"), admission("viewer"), mode).await;
    sfu.shutdown().await;
    http.abort();
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

struct Backend {
    child: std::process::Child,
    _dir: tempfile::TempDir,
    base: String,
}
impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Backend {
    async fn start() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let dir = tempfile::tempdir().unwrap();
        let path = std::env::var("PINGO_SERVER").unwrap_or_else(|_| {
            format!(
                "{}/../../target/debug/pingo-server",
                env!("CARGO_MANIFEST_DIR")
            )
        });
        let child = std::process::Command::new(path)
            .args([
                "serve",
                "--dev",
                "--http",
                &address.to_string(),
                "--dir",
                dir.path().to_str().unwrap(),
                "--codegen-out",
                dir.path().join("codegen").to_str().unwrap(),
            ])
            .env("PINGO_PUBLIC_IPS", "127.0.0.1")
            .env("PINGO_UDP_PORT", "0")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("cargo build -p pingo-server first");
        let backend = Self {
            child,
            _dir: dir,
            base: format!("http://{address}"),
        };
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if reqwest::get(format!("{}/api/health", backend.base))
                    .await
                    .is_ok_and(|r| r.status().is_success())
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        backend
    }
    async fn post(
        &self,
        path: &str,
        value: serde_json::Value,
        access: Option<&str>,
    ) -> serde_json::Value {
        let mut request = reqwest::Client::new()
            .post(format!("{}{path}", self.base))
            .json(&value);
        if let Some(access) = access {
            request = request.bearer_auth(access);
        }
        request.send().await.unwrap().json().await.unwrap()
    }
    async fn call(
        &self,
        kind: &str,
        path: &str,
        args: serde_json::Value,
        access: Option<&str>,
    ) -> serde_json::Value {
        let reply = self
            .post(
                &format!("/api/{kind}"),
                serde_json::json!({"path":path,"args":args}),
                access,
            )
            .await;
        assert_eq!(reply["status"], "success", "{reply}");
        reply["value"].clone()
    }
    async fn access(&self, email: &str) -> String {
        self.post(
            "/api/auth/magic/request",
            serde_json::json!({"email":email}),
            None,
        )
        .await;
        let code = self
            .call(
                "action",
                "dev:last_code",
                serde_json::json!({"email":email}),
                None,
            )
            .await;
        self.post(
            "/api/auth/magic/verify",
            serde_json::json!({"email":email,"code":code}),
            None,
        )
        .await["accessToken"]
            .as_str()
            .unwrap()
            .into()
    }
}
#[tokio::test]
#[ignore = "requires cargo build -p pingo-server; runs the real RxFerrix executable"]
async fn native_video_full_server_text_and_motion() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let server = Backend::start().await;
            let space = server
                .call("mutation", "dev:seed", serde_json::json!({}), None)
                .await;
            let a = server.access("alex@pingo.test").await;
            let b = server.access("sam@pingo.test").await;
            let rooms = server
                .call(
                    "query",
                    "rooms:list",
                    serde_json::json!({"space":space}),
                    Some(&a),
                )
                .await;
            let room = rooms
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["name"] == "Lounge")
                .unwrap()["_id"]
                .clone();
            for mode in [VideoMode::Text, VideoMode::Motion] {
                let mut tokens = Vec::new();
                for access in [&a, &b] {
                    tokens.push(
                        server
                            .call(
                                "mutation",
                                "voice:join",
                                serde_json::json!({"room":room}),
                                Some(access),
                            )
                            .await["token"]
                            .as_str()
                            .unwrap()
                            .to_owned(),
                    );
                }
                exercise(
                    &format!("{}/rtc", server.base.replace("http://", "ws://")),
                    tokens.remove(0),
                    tokens.remove(0),
                    mode,
                )
                .await;
            }
        })
        .await;
}
