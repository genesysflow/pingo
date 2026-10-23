//! Real HTTP auth/functions and localhost ICE/DTLS/SRTP through the embedded route.
use super::*;
use crate::model::{Room, Space};
use crate::{invites, rooms, spaces};
use futures_util::{SinkExt, StreamExt};
use pingo_proto::signal::PublishedTrack;
use pingo_proto::{ClientMsg, ServerMsg, TrackKind, token};
use rxferrix::Id;
use rxferrix::auth::User;
use rxferrix::testing::TestApp;
use serde_json::{Value, json};
use std::time::Instant;
use str0m::change::{SdpAnswer, SdpOffer};
use str0m::media::{Direction, Frequency, MediaKind, MediaTime};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

const OPUS: &[u8] = &[0xf8, 0xff, 0xfe];

struct Server {
    t: TestApp,
    media: Media,
    address: SocketAddr,
    http: tokio::task::JoinHandle<()>,
    ann: Id<User>,
    bob: Id<User>,
    space: Id<Space>,
    room: Id<Room>,
    ann_auth: String,
    bob_auth: String,
}

impl Server {
    async fn start(dev: bool) -> Self {
        let t = TestApp::with_options(rxferrix::AppOptions {
            dev,
            ..Default::default()
        })
        .await
        .unwrap();
        let ann = t.create_user("ann@example.com", None).await.unwrap();
        let bob = t.create_user("bob@example.com", None).await.unwrap();
        let space = t
            .as_user(ann)
            .mutation::<spaces::create>(("Media".into(), None))
            .await
            .unwrap();
        let invite = t
            .as_user(ann)
            .mutation::<invites::create>((space, None, None))
            .await
            .unwrap();
        t.as_user(bob)
            .mutation::<invites::accept>((invite,))
            .await
            .unwrap();
        let room = t.as_user(ann).query::<rooms::list>((space,)).await.unwrap()[0].id;
        let ann_auth = t
            .app()
            .auth()
            .sign_in_as(t.app(), ann)
            .await
            .unwrap()
            .access_token;
        let bob_auth = t
            .app()
            .auth()
            .sign_in_as(t.app(), bob)
            .await
            .unwrap()
            .access_token;
        let media = Media::new(SfuConfig {
            udp_bind: "127.0.0.1:0".parse().unwrap(),
            ..Default::default()
        });
        let router = rxferrix::ops::web::router(
            t.app().clone(),
            &rxferrix::ops::WebOptions {
                routes: Some(media.clone().router()),
                ..Default::default()
            },
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let http = tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        Self {
            t,
            media,
            address,
            http,
            ann,
            bob,
            space,
            room,
            ann_auth,
            bob_auth,
        }
    }

    async fn call(&self, kind: &str, path: &str, args: Value, auth: &str) -> Value {
        let reply: Value = reqwest::Client::new()
            .post(format!("http://{}/api/{kind}", self.address))
            .bearer_auth(auth)
            .json(&json!({"path": path, "args": args}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(reply["status"], "success", "{reply}");
        reply["value"].clone()
    }

    async fn token(&self, auth: &str) -> String {
        self.call("mutation", "voice:join", json!({"room": self.room}), auth)
            .await["token"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    async fn secret(&self, secret: Option<&str>) {
        // Same env mutation used by the dashboard, with the admin check bypassed
        // by a trusted typed call rather than any test-only server endpoint.
        self.t
            .mutation::<rxferrix::env::put>((
                "PINGO_MEDIA_SECRET".into(),
                secret.map(str::to_owned),
            ))
            .await
            .unwrap();
    }

    async fn participants(&self) -> Value {
        self.call(
            "query",
            "voice:participants",
            json!({"room": self.room}),
            &self.ann_auth,
        )
        .await
    }

    async fn wait_participants(&self, predicate: impl Fn(&[Value]) -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let rows = self.participants().await;
                if predicate(rows.as_array().unwrap()) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("voice hooks did not reach the database");
    }

    async fn stop(self) {
        let sfu = self.media.sfu.get().unwrap().clone();
        self.t.app().begin_close();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if sfu.stats().participants == 0
                    && self.participants().await.as_array().unwrap().is_empty()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        // Also join the hook worker, avoiding races with temporary-directory drop.
        sfu.shutdown().await;
        self.http.abort();
        let _ = self.http.await;
    }
}

async fn websocket(
    address: SocketAddr,
    token: String,
) -> (
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    ServerMsg,
) {
    let (mut ws, reply) = tokio_tungstenite::connect_async(format!("ws://{address}/rtc"))
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::SWITCHING_PROTOCOLS);
    ws.send(Message::Text(
        serde_json::to_string(&ClientMsg::Join { token })
            .unwrap()
            .into(),
    ))
    .await
    .unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let reply = serde_json::from_str(reply.to_text().unwrap()).unwrap();
    (ws, reply)
}

#[derive(Debug)]
enum ClientEvent {
    Connected,
    Signal(ServerMsg),
    Audio(Vec<u8>),
}

struct Client {
    control: mpsc::UnboundedSender<Option<ClientMsg>>,
    events: mpsc::UnboundedReceiver<ClientEvent>,
    task: tokio::task::JoinHandle<()>,
}

impl Client {
    // Audio-only version of the SFU tests' sans-IO peer, using a backend-issued token.
    async fn join(server: &Server, token: String, audio: bool) -> Self {
        let (mut ws, reply) = websocket(server.address, token).await;
        assert!(matches!(reply, ServerMsg::Joined { .. }), "{reply:?}");
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let mut rtc = Rtc::builder()
            .set_crypto_provider(Arc::new(str0m::crypto::from_feature_flags()))
            .clear_codecs()
            .enable_opus(true, true)
            .build(Instant::now());
        rtc.add_local_candidate(Candidate::host(address, "udp").unwrap());
        let mut change = rtc.sdp_api();
        let mid = audio
            .then(|| change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None));
        change.add_channel("pingo".into());
        let (offer, initial_pending) = change.apply().unwrap();
        for message in [
            ClientMsg::Offer {
                sdp: offer.to_sdp_string(),
            },
            ClientMsg::PublishTracks {
                tracks: mid
                    .into_iter()
                    .map(|mid| PublishedTrack {
                        mid: mid.to_string(),
                        kind: TrackKind::Audio,
                    })
                    .collect(),
            },
        ] {
            ws.send(Message::Text(
                serde_json::to_string(&message).unwrap().into(),
            ))
            .await
            .unwrap();
        }
        let (control, mut commands) = mpsc::unbounded_channel();
        let (tx, events) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut pending = Some(initial_pending);
            let mut buffer = vec![0; 65536];
            let mut connected = false;
            let mut samples = 0;
            let mut audio_at = Instant::now();
            loop {
                let now = Instant::now();
                rtc.handle_input(Input::Timeout(now)).unwrap();
                if connected && now >= audio_at {
                    if let Some(mid) = mid
                        && let Some(writer) = rtc.writer(mid)
                    {
                        let pt = writer.payload_params().next().unwrap().pt();
                        writer
                            .write(
                                pt,
                                now,
                                MediaTime::new(samples, Frequency::FORTY_EIGHT_KHZ),
                                OPUS.to_vec(),
                            )
                            .unwrap();
                        samples += 960;
                    }
                    audio_at = now + Duration::from_millis(20);
                }
                let next = loop {
                    match rtc.poll_output().unwrap() {
                        Output::Timeout(at) => {
                            break at.min(Instant::now() + Duration::from_millis(10));
                        }
                        Output::Transmit(packet) => {
                            socket
                                .send_to(&packet.contents, packet.destination)
                                .await
                                .unwrap();
                        }
                        Output::Event(Event::Connected) => {
                            connected = true;
                            let _ = tx.send(ClientEvent::Connected);
                        }
                        Output::Event(Event::MediaData(data)) => {
                            let _ = tx.send(ClientEvent::Audio(data.data.to_vec()));
                        }
                        _ => {}
                    }
                };
                tokio::select! {
                    frame = ws.next() => {
                        let message = match frame {
                            Some(Ok(Message::Text(text))) => serde_json::from_str::<ServerMsg>(&text).unwrap(),
                            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                            _ => continue,
                        };
                        match &message {
                            ServerMsg::Answer { sdp } => rtc.sdp_api().accept_answer(pending.take().unwrap(), SdpAnswer::from_sdp_string(sdp).unwrap()).unwrap(),
                            ServerMsg::Offer { sdp } => {
                                let answer = rtc.sdp_api().accept_offer(SdpOffer::from_sdp_string(sdp).unwrap()).unwrap();
                                ws.send(Message::Text(serde_json::to_string(&ClientMsg::Answer { sdp: answer.to_sdp_string() }).unwrap().into())).await.unwrap();
                            }
                            _ => {}
                        }
                        let _ = tx.send(ClientEvent::Signal(message));
                    }
                    packet = socket.recv_from(&mut buffer) => {
                        let (len, source) = packet.unwrap();
                        rtc.handle_input(Input::Receive(Instant::now(), Receive { proto: Protocol::Udp, source, destination: address, contents: buffer[..len].try_into().unwrap() })).unwrap();
                    }
                    command = commands.recv() => match command {
                        Some(Some(message)) => { ws.send(Message::Text(serde_json::to_string(&message).unwrap().into())).await.unwrap(); }
                        _ => { let _ = ws.close(None).await; break; }
                    },
                    _ = tokio::time::sleep_until(next.into()) => {}
                }
            }
        });
        Self {
            control,
            events,
            task,
        }
    }

    async fn wait(&mut self, predicate: impl Fn(&ClientEvent) -> bool) {
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                let event = self.events.recv().await.expect("peer stopped");
                if let ClientEvent::Signal(ServerMsg::Error { code, message }) = &event {
                    panic!("{code}: {message}");
                }
                if predicate(&event) {
                    break;
                }
            }
        })
        .await
        .expect("WebRTC event timed out");
    }

    async fn stop(self) {
        let _ = self.control.send(None);
        self.task.await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embedded_media_updates_queries_forwards_audio_and_cleans_up() {
    let server = Server::start(true).await;
    // Simulate crash residue for a different user, then exercise first-request cleanup.
    server
        .t
        .mutation::<voice::set_state>((server.room, server.bob, false, false, false, false))
        .await
        .unwrap();
    let ann_token = server.token(&server.ann_auth).await;
    let mut ann = Client::join(&server, ann_token, true).await;
    ann.wait(|e| matches!(e, ClientEvent::Connected)).await;
    server
        .wait_participants(|rows| rows.len() == 1 && rows[0]["user"] == server.ann.to_string())
        .await;
    let bob_token = server.token(&server.bob_auth).await;
    let mut bob = Client::join(&server, bob_token, false).await;
    bob.wait(|e| matches!(e, ClientEvent::Signal(ServerMsg::TrackPublished { track }) if track.kind == TrackKind::Audio)).await;
    bob.wait(|e| matches!(e, ClientEvent::Audio(data) if data == OPUS))
        .await;
    server.wait_participants(|rows| rows.len() == 2).await;
    ann.control
        .send(Some(ClientMsg::SetState {
            muted: true,
            deafened: true,
        }))
        .unwrap();
    server
        .wait_participants(|rows| {
            rows.iter().any(|r| {
                r["user"] == server.ann.to_string() && r["muted"] == true && r["deafened"] == true
            })
        })
        .await;
    let rooms = server
        .call(
            "query",
            "rooms:list",
            json!({"space": server.space}),
            &server.bob_auth,
        )
        .await;
    let room = rooms
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["_id"] == server.room.to_string())
        .unwrap();
    assert_eq!(room["voice"].as_array().unwrap().len(), 2);
    assert!(
        room["voice"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["user"] == server.ann.to_string() && r["muted"] == true)
    );
    ann.stop().await;
    server
        .wait_participants(|rows| rows.len() == 1 && rows[0]["user"] == server.bob.to_string())
        .await;
    // Runtime close must disconnect the remaining peer and drain its leave hook.
    server.stop().await;
    bob.task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admission_tracks_secret_rotation_removal_and_rejects_wrong_keys() {
    let server = Server::start(true).await;
    let dev_token = server.token(&server.ann_auth).await;
    let claims = token::verify(&dev_token, voice::DEV_MEDIA_SECRET.as_bytes(), 0.0).unwrap();
    let (mut ws, reply) = websocket(server.address, token::sign(&claims, b"wrong key")).await;
    assert!(matches!(reply, ServerMsg::Error { code, .. } if code == "invalid_token"));
    let _ = ws.close(None).await;
    // Rotate twice, rejecting the old token immediately each time.
    let mut previous = dev_token;
    for secret in ["dashboard key one", "dashboard key two"] {
        server.secret(Some(secret)).await;
        let (mut ws, reply) = websocket(server.address, previous).await;
        assert!(matches!(reply, ServerMsg::Error { code, .. } if code == "invalid_token"));
        let _ = ws.close(None).await;
        previous = server.token(&server.ann_auth).await;
        token::verify(&previous, secret.as_bytes(), 0.0).unwrap();
        let peer = Client::join(&server, previous.clone(), false).await;
        peer.stop().await;
    }
    // The key-returning query and reset mutation must stay off the public API,
    // even for an authenticated member.
    for (kind, path) in [
        ("query", "voice:admission_secret"),
        ("mutation", "voice:reset"),
    ] {
        let reply: Value = reqwest::Client::new()
            .post(format!("http://{}/api/{kind}", server.address))
            .bearer_auth(&server.ann_auth)
            .json(&json!({"path": path, "args": {}}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(reply["status"], "error");
        assert!(reply.get("value").is_none());
    }
    server.secret(Some("")).await;
    let dev = server.token(&server.ann_auth).await;
    token::verify(&dev, voice::DEV_MEDIA_SECRET.as_bytes(), 0.0).unwrap();
    let peer = Client::join(&server, dev, false).await;
    peer.stop().await;
    server.secret(None).await;
    let peer = Client::join(&server, server.token(&server.ann_auth).await, false).await;
    peer.stop().await;
    server.stop().await;
}

#[tokio::test]
async fn production_admission_fails_closed_when_secret_is_missing_or_removed() {
    let server = Server::start(false).await;
    let dev = Server::start(true).await;
    let dev_token = dev.token(&dev.ann_auth).await;
    for value in [None, Some("")] {
        server.secret(value).await;
        let (mut ws, reply) = websocket(server.address, dev_token.clone()).await;
        assert!(matches!(reply, ServerMsg::Error { code, .. } if code == "invalid_token"));
        let _ = ws.close(None).await;
    }
    server.secret(Some("production key")).await;
    let token = server.token(&server.ann_auth).await;
    let peer = Client::join(&server, token.clone(), false).await;
    peer.stop().await;
    server.secret(None).await;
    let (mut ws, reply) = websocket(server.address, token).await;
    assert!(matches!(reply, ServerMsg::Error { code, .. } if code == "invalid_token"));
    let _ = ws.close(None).await;
    server.stop().await;
    // Start the other instance before shutting it down (its media is lazy).
    let (mut ws, _) = websocket(dev.address, dev_token).await;
    let _ = ws.close(None).await;
    dev.stop().await;
}

mod native;
