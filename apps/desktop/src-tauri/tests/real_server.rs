//! Explicit localhost integration: build pingo-server, then run with --ignored.
use pingo_desktop::calls::{CallManager, JoinRequest, Settings};
use pingo_media::rtc::{Client, ClientConfig, Command};
use serde_json::{Value, json};
use std::{
    process::{Child, Command as Process, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
async fn function(
    client: &reqwest::Client,
    origin: &str,
    auth: &str,
    kind: &str,
    path: &str,
    args: Value,
) -> Value {
    let response: Value = client
        .post(format!("{origin}/api/{kind}"))
        .bearer_auth(auth)
        .json(&json!({"path":path,"args":args}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(response["status"], "success", "{response}");
    response["value"].clone()
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "starts the built real server on localhost; run after cargo build -p pingo-server"]
async fn native_manager_receives_real_remote_audio_and_durable_participants() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let directory = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let origin = format!("http://{address}");
    let binary = std::env::var_os("PINGO_SERVER")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("target/debug/pingo-server"));
    let _server = Server(
        Process::new(binary)
            .args(["serve", "--dev", "--http", &address.to_string(), "--dir"])
            .arg(directory.path())
            .arg("--codegen-out")
            .arg(directory.path().join("codegen"))
            .env("PINGO_UDP_PORT", "0")
            .env("PINGO_PUBLIC_IPS", "127.0.0.1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("build pingo-server first"),
    );
    let http = reqwest::Client::new();
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if http
                .get(format!("{origin}/api/health"))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    let mut auth = Vec::new();
    for name in ["Desktop", "NativePeer"] {
        let response:Value=http.post(format!("{origin}/api/auth/signup")).json(&json!({"name":name,"email":format!("{}@pingo.test",name.to_lowercase()),"password":"correct-horse-pingo"})).send().await.unwrap().json().await.unwrap();
        auth.push(
            response["accessToken"]
                .as_str()
                .expect("signup token")
                .to_owned(),
        );
    }
    let space = function(
        &http,
        &origin,
        &auth[0],
        "mutation",
        "spaces:create",
        json!({"name":"Native smoke"}),
    )
    .await;
    let invite = function(
        &http,
        &origin,
        &auth[0],
        "mutation",
        "invites:create",
        json!({"space":space}),
    )
    .await;
    function(
        &http,
        &origin,
        &auth[1],
        "mutation",
        "invites:accept",
        json!({"code":invite}),
    )
    .await;
    let rooms = function(
        &http,
        &origin,
        &auth[0],
        "query",
        "rooms:list",
        json!({"space":space}),
    )
    .await;
    let room = rooms[0]["_id"].clone();
    let mut tokens = Vec::new();
    for token in &auth {
        tokens.push(
            function(
                &http,
                &origin,
                token,
                "mutation",
                "voice:join",
                json!({"room":room}),
            )
            .await["token"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    let manager = CallManager::default();
    let (events, mut receive) = mpsc::unbounded_channel();
    manager
        .join(
            JoinRequest {
                id: "desktop-smoke".into(),
                origin: &origin,
                url: "/rtc",
                token: tokens[0].clone(),
                settings: Settings::default(),
                video_sink: None,
            },
            Arc::new(move |event| {
                let _ = events.send(event);
            }),
            false,
        )
        .unwrap();
    let mut config = ClientConfig::new(format!("ws://{address}/rtc"), tokens[1].clone());
    config.settings.noise_level = pingo_media::audio::denoise::NoiseLevel::Off;
    let peer = Client::connect(config).await.unwrap();
    let (commands, rx) = mpsc::channel(32);
    let run = peer.run(None, rx);
    tokio::pin!(run);
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut offset = 0;
    let mut pcm = 0;
    let mut packets = 0;
    let mut remote = false;
    loop {
        assert!(
            Instant::now() < deadline,
            "no remote audio: pcm={pcm}, packets={packets}, remote={remote}"
        );
        tokio::select! {
            result=&mut run=>panic!("peer stopped early: {result:?}"),
            _=tick.tick()=>{
                let frame=std::array::from_fn(|i| { let sample=0.15*(std::f32::consts::TAU*440.0*(offset+i) as f32/48000.0).sin(); [sample,sample] }); offset+=480;
                commands.try_send(Command::Capture(Box::new(frame))).unwrap();
            },
            event=receive.recv()=>{
                let event=event.unwrap().event;
                match event["type"].as_str().unwrap() {
                    "error"=>panic!("native manager: {event}"),
                    "participantJoined"=>remote=true,
                    "joined"=>remote|=event["participants"].as_array().unwrap().iter().any(|p|p["name"]=="NativePeer"),
                    "playback"=>pcm=event["receivedPcmBytes"].as_u64().unwrap(),
                    "playout"=>packets=event["receivedPackets"].as_u64().unwrap(),
                    _=>{}
                }
            }
        }
        if pcm > 0 && packets > 0 && remote {
            break;
        }
    }
    let participants = function(
        &http,
        &origin,
        &auth[0],
        "query",
        "voice:participants",
        json!({"room":room}),
    )
    .await;
    assert_eq!(participants.as_array().unwrap().len(), 2, "{participants}");
    println!(
        "Native smoke: {pcm} non-silent decoded PCM bytes; {packets} remote Opus packets; 2 durable participants"
    );
    manager.leave(None);
    commands.send(Command::Leave).await.unwrap();
    run.await.unwrap();
}
