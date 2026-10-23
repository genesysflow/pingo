//! Real localhost WebSocket + ICE/DTLS/SRTP/SCTP interop, without audio hardware.
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use pingo_proto::signal::PublishedTrack;
use pingo_proto::{ClientMsg, DataMsg, MediaClaims, ServerMsg, TrackKind, token};
use pingo_sfu::{Sfu, SfuConfig, VoiceHooks};
use str0m::change::{SdpAnswer, SdpOffer};
use str0m::format::Codec;
use str0m::media::{Direction, Frequency, MediaKind, MediaTime, Mid, Simulcast, SimulcastLayer};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

const SECRET: &[u8] = b"localhost integration secret";
const OPUS: &[u8] = &[0xf8, 0xff, 0xfe];

#[derive(Debug)]
enum HookEvent {
    State(String, String, bool, bool, bool, bool),
    Leave(String, String),
}

struct Hooks(mpsc::UnboundedSender<HookEvent>);
#[async_trait]
impl VoiceHooks for Hooks {
    async fn on_state(
        &self,
        room: &str,
        user: &str,
        muted: bool,
        deafened: bool,
        sharing_screen: bool,
        video: bool,
    ) {
        let _ = self.0.send(HookEvent::State(
            room.into(),
            user.into(),
            muted,
            deafened,
            sharing_screen,
            video,
        ));
    }
    async fn on_leave(&self, room: &str, user: &str) {
        let _ = self.0.send(HookEvent::Leave(room.into(), user.into()));
    }
}

struct Server {
    sfu: Sfu,
    address: SocketAddr,
    http: tokio::task::JoinHandle<()>,
    hooks: mpsc::UnboundedReceiver<HookEvent>,
}
impl Server {
    async fn start() -> Self {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init();
        let (tx, hooks) = mpsc::unbounded_channel();
        let sfu = Sfu::start(
            SfuConfig {
                media_secret: SECRET.to_vec(),
                udp_bind: "127.0.0.1:0".parse().unwrap(),
                public_ips: vec![],
                ..Default::default()
            },
            Arc::new(Hooks(tx)),
        )
        .await
        .unwrap();
        // Verify the exact generic-state merge required by RxFerrix without depending on it.
        #[derive(Clone)]
        struct App;
        let router: axum::Router<App> = axum::Router::new().merge(sfu.router());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let http = tokio::spawn(async move {
            axum::serve(listener, router.with_state(App)).await.unwrap();
        });
        Self {
            sfu,
            address,
            http,
            hooks,
        }
    }
    async fn stop(self) {
        self.sfu.shutdown().await;
        self.http.abort();
    }
}

fn claims(user: &str, room: &str) -> MediaClaims {
    MediaClaims {
        room: room.into(),
        user: user.into(),
        name: user.into(),
        color: "#f97316".into(),
        can_speak: true,
        can_video: true,
        can_share: true,
        exp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64
            + 60_000.0,
    }
}

#[derive(Clone, Debug)]
enum ClientEvent {
    Signal(ServerMsg),
    Connected,
    Channel,
    Media {
        mid: Mid,
        data: Vec<u8>,
        level: Option<i8>,
    },
    Data(DataMsg),
    Keyframe,
    Closed,
}

enum Control {
    Data(DataMsg),
    Signal(ClientMsg),
    Stop,
}

struct Client {
    id: String,
    control: mpsc::UnboundedSender<Control>,
    events: mpsc::UnboundedReceiver<ClientEvent>,
    saved: Vec<ClientEvent>,
    task: tokio::task::JoinHandle<()>,
}

impl Client {
    async fn join(address: SocketAddr, claims: MediaClaims, audio: bool) -> Self {
        Self::join_media(address, claims, audio, None).await
    }

    async fn join_media(
        address: SocketAddr,
        claims: MediaClaims,
        audio: bool,
        video: Option<(TrackKind, Codec)>,
    ) -> Self {
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{address}/rtc"))
            .await
            .unwrap();
        ws.send(Message::Text(
            serde_json::to_string(&ClientMsg::Join {
                token: token::sign(&claims, SECRET),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
        let joined: ServerMsg =
            serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        let ServerMsg::Joined {
            participant_id: id, ..
        } = &joined
        else {
            panic!("{joined:?}");
        };
        let id = id.clone();
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = socket.local_addr().unwrap();
        let mut rtc = Rtc::builder()
            .set_crypto_provider(Arc::new(str0m::crypto::from_feature_flags()))
            .clear_codecs()
            .enable_opus(true, true)
            .enable_vp8(true)
            .enable_h264(true)
            .build(Instant::now());
        rtc.add_local_candidate(Candidate::host(addr, "udp").unwrap());
        let mut change = rtc.sdp_api();
        let audio_mid = audio
            .then(|| change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None));
        let video_mid = video.map(|_| {
            let mut simulcast = Simulcast::new();
            simulcast.add_send_layer(
                SimulcastLayer::new_with_attributes("q")
                    .max_height(180)
                    .max_br(50_000)
                    .build(),
            );
            simulcast.add_send_layer(
                SimulcastLayer::new_with_attributes("h")
                    .max_height(720)
                    .max_br(150_000)
                    .build(),
            );
            change.add_media(
                MediaKind::Video,
                Direction::SendOnly,
                None,
                None,
                Some(simulcast),
            )
        });
        let cid = change.add_channel("pingo".into());
        let (offer, initial_pending) = change.apply().unwrap();
        ws.send(Message::Text(
            serde_json::to_string(&ClientMsg::Offer {
                sdp: offer.to_sdp_string(),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
        let mut tracks: Vec<_> = audio_mid
            .map(|mid| PublishedTrack {
                mid: mid.to_string(),
                kind: TrackKind::Audio,
            })
            .into_iter()
            .collect();
        if let Some(mid) = video_mid {
            tracks.push(PublishedTrack {
                mid: mid.to_string(),
                kind: video.unwrap().0,
            });
        }
        ws.send(Message::Text(
            serde_json::to_string(&ClientMsg::PublishTracks { tracks })
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
        let (control, mut rx) = mpsc::unbounded_channel();
        let (events_tx, events) = mpsc::unbounded_channel();
        events_tx.send(ClientEvent::Signal(joined)).unwrap();
        let task = tokio::spawn(async move {
            let mut pending = Some(initial_pending);
            let mut buffer = vec![0; 65536];
            let mut connected = false;
            let mut channel_open = false;
            let mut samples = 0;
            let mut audio_at = Instant::now();
            loop {
                let now = Instant::now();
                rtc.handle_input(Input::Timeout(now)).unwrap();
                if connected && now >= audio_at {
                    if let Some(mid) = audio_mid
                        && let Some(writer) = rtc.writer(mid)
                    {
                        let pt = writer.payload_params().next().unwrap().pt();
                        writer
                            .audio_level(-10, true)
                            .write(
                                pt,
                                now,
                                MediaTime::new(samples, Frequency::FORTY_EIGHT_KHZ),
                                OPUS.to_vec(),
                            )
                            .unwrap();
                        samples += 960;
                    }
                    if let Some(mid) = video_mid {
                        let codec = video.unwrap().1;
                        for (rid, marker) in [("q", 42), ("h", 84)] {
                            if let Some(writer) = rtc.writer(mid) {
                                let pt = writer
                                    .payload_params()
                                    .find(|p| p.spec().codec == codec)
                                    .unwrap()
                                    .pt();
                                // Synthetic keyframe headers; no decoder or camera needed.
                                let data = if codec == Codec::Vp8 {
                                    vec![0, 0, 0, marker]
                                } else {
                                    vec![0, 0, 0, 1, 0x65, marker]
                                };
                                writer
                                    .rid(rid.into())
                                    .write(pt, now, MediaTime::from_millis(samples), data)
                                    .unwrap();
                            }
                        }
                        samples += 20;
                    }
                    audio_at = now + Duration::from_millis(20);
                }
                let next = loop {
                    match rtc.poll_output().unwrap() {
                        Output::Timeout(at) => {
                            break at.min(Instant::now() + Duration::from_millis(10));
                        }
                        Output::Transmit(tx) => {
                            socket.send_to(&tx.contents, tx.destination).await.unwrap();
                        }
                        Output::Event(Event::Connected) => {
                            connected = true;
                            let _ = events_tx.send(ClientEvent::Connected);
                        }
                        Output::Event(Event::ChannelOpen(open_id, label)) if label == "pingo" => {
                            assert_eq!(open_id, cid);
                            channel_open = true;
                            let _ = events_tx.send(ClientEvent::Channel);
                        }
                        Output::Event(Event::MediaData(data)) => {
                            let _ = events_tx.send(ClientEvent::Media {
                                mid: data.mid,
                                data: data.data.to_vec(),
                                level: data.ext_vals.audio_level,
                            });
                        }
                        Output::Event(Event::KeyframeRequest(_)) => {
                            let _ = events_tx.send(ClientEvent::Keyframe);
                        }
                        Output::Event(Event::ChannelData(data)) => {
                            if let Ok(data) = serde_json::from_slice(&data.data) {
                                let _ = events_tx.send(ClientEvent::Data(data));
                            }
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
                            ServerMsg::Answer { sdp } => { rtc.sdp_api().accept_answer(pending.take().expect("pending client offer"), SdpAnswer::from_sdp_string(sdp).unwrap()).unwrap(); }
                            ServerMsg::Offer { sdp } => {
                                let answer = rtc.sdp_api().accept_offer(SdpOffer::from_sdp_string(sdp).unwrap()).unwrap();
                                ws.send(Message::Text(serde_json::to_string(&ClientMsg::Answer { sdp: answer.to_sdp_string() }).unwrap().into())).await.unwrap();
                            }
                            _ => {}
                        }
                        let _ = events_tx.send(ClientEvent::Signal(message));
                    }
                    packet = socket.recv_from(&mut buffer) => {
                        let (len, source) = packet.unwrap();
                        rtc.handle_input(Input::Receive(Instant::now(), Receive { proto: Protocol::Udp, source, destination: addr, contents: buffer[..len].try_into().unwrap() })).unwrap();
                    }
                    command = rx.recv() => match command {
                        Some(Control::Data(data)) => { assert!(channel_open); assert!(rtc.channel(cid).unwrap().write(false, &serde_json::to_vec(&data).unwrap()).unwrap()); }
                        Some(Control::Signal(message)) => { ws.send(Message::Text(serde_json::to_string(&message).unwrap().into())).await.unwrap(); }
                        Some(Control::Stop) | None => { let _ = ws.close(None).await; break; }
                    },
                    _ = tokio::time::sleep_until(next.into()) => {}
                }
            }
            let _ = events_tx.send(ClientEvent::Closed);
        });
        Self {
            id,
            control,
            events,
            saved: Vec::new(),
            task,
        }
    }

    async fn wait(&mut self, predicate: impl Fn(&ClientEvent) -> bool) -> ClientEvent {
        if let Some(index) = self.saved.iter().position(&predicate) {
            return self.saved.remove(index);
        }
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                let event = self.events.recv().await.expect("client task stopped");
                if predicate(&event) {
                    return event;
                }
                if let ClientEvent::Signal(ServerMsg::Error { code, message }) = &event {
                    panic!("server error {code}: {message}");
                }
                self.saved.push(event);
            }
        })
        .await
        .unwrap_or_else(|_| panic!("client {} timed out; events: {:?}", self.id, self.saved))
    }
    async fn ready(&mut self) {
        self.wait(|e| matches!(e, ClientEvent::Connected)).await;
        self.wait(|e| matches!(e, ClientEvent::Channel)).await;
    }
    async fn stop(self) {
        let _ = self.control.send(Control::Stop);
        self.task.await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forwards_opus_with_track_mapping_and_cleans_up_on_leave() {
    let mut server = Server::start().await;
    let mut a = Client::join(server.address, claims("alice", "room"), true).await;
    a.ready().await;
    let mut b = Client::join(server.address, claims("bob", "room"), false).await;
    let joined = b
        .wait(|e| matches!(e, ClientEvent::Signal(ServerMsg::Joined { .. })))
        .await;
    let ClientEvent::Signal(ServerMsg::Joined { participants, .. }) = joined else {
        unreachable!()
    };
    assert_eq!(participants.len(), 1);
    assert_eq!(participants[0].participant_id, a.id);
    let b_id = b.id.clone();
    a.wait(|e| matches!(e, ClientEvent::Signal(ServerMsg::ParticipantJoined { participant }) if participant.participant_id == b_id)).await;
    b.ready().await;
    let a_id = a.id.clone();
    let published = b.wait(|e| matches!(e, ClientEvent::Signal(ServerMsg::TrackPublished { track }) if track.participant_id == a_id && track.kind == TrackKind::Audio)).await;
    let ClientEvent::Signal(ServerMsg::TrackPublished { track }) = published else {
        unreachable!()
    };
    b.wait(|e| matches!(e, ClientEvent::Media { mid, data, level } if mid.to_string() == track.mid && data == OPUS && *level == Some(-10))).await;
    assert_eq!(server.sfu.stats().rooms, 1);
    assert_eq!(server.sfu.stats().participants, 2);
    assert_eq!(server.sfu.stats().tracks, 1);
    assert!(server.sfu.stats().bytes_in > 0 && server.sfu.stats().bytes_out > 0);
    a.stop().await;
    b.wait(|e| matches!(e, ClientEvent::Signal(ServerMsg::ParticipantLeft { participant_id }) if *participant_id == a_id)).await;
    b.wait(|e| matches!(e, ClientEvent::Signal(ServerMsg::TrackUnpublished { participant_id, mid }) if *participant_id == a_id && *mid == track.mid)).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop { if matches!(server.hooks.recv().await, Some(HookEvent::Leave(room, user)) if room == "room" && user == "alice") { break; } }
    }).await.unwrap();
    b.stop().await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relays_data_channel_with_real_sender_identity_and_speaking_levels() {
    let server = Server::start().await;
    let mut a = Client::join(server.address, claims("alice", "room"), true).await;
    let mut b = Client::join(server.address, claims("bob", "room"), false).await;
    a.ready().await;
    b.ready().await;
    a.control
        .send(Control::Data(DataMsg::Reaction {
            participant_id: "forged".into(),
            emoji: "🎉".into(),
        }))
        .unwrap();
    let a_id = a.id.clone();
    b.wait(|e| matches!(e, ClientEvent::Data(DataMsg::Reaction { participant_id, emoji }) if *participant_id == a_id && emoji == "🎉")).await;
    b.wait(|e| matches!(e, ClientEvent::Data(DataMsg::Speaking { participant_id, level }) if *participant_id == a_id && *level > 0.5)).await;
    a.stop().await;
    b.stop().await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replaces_duplicate_sessions_and_delivers_state_hooks_in_order() {
    let mut server = Server::start().await;
    let mut a = Client::join(server.address, claims("alice", "room"), false).await;
    a.ready().await;
    let old_id = a.id.clone();
    let mut observer = Client::join(server.address, claims("bob", "room"), false).await;
    observer.ready().await;
    a.control
        .send(Control::Signal(ClientMsg::SetState {
            muted: true,
            deafened: true,
        }))
        .unwrap();
    observer.wait(|e| matches!(e, ClientEvent::Signal(ServerMsg::ParticipantUpdated { participant }) if participant.participant_id == old_id && participant.muted && participant.deafened)).await;
    let mut replacement = Client::join(server.address, claims("alice", "room"), false).await;
    replacement.ready().await;
    observer.wait(|e| matches!(e, ClientEvent::Signal(ServerMsg::ParticipantLeft { participant_id }) if *participant_id == old_id)).await;
    assert_ne!(replacement.id, old_id);
    assert_eq!(server.sfu.stats().participants, 2);
    let mut history = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), async {
        while history.len() < 4 {
            match server.hooks.recv().await.unwrap() {
                HookEvent::State(room, user, muted, deafened, sharing, video)
                    if user == "alice" =>
                {
                    assert_eq!(room, "room");
                    assert!(!sharing && !video);
                    history.push((muted, deafened, false));
                }
                HookEvent::Leave(room, user) if user == "alice" => {
                    assert_eq!(room, "room");
                    history.push((false, false, true));
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        history,
        vec![
            (false, false, false),
            (true, true, false),
            (false, false, true),
            (false, false, false)
        ]
    );
    a.stop().await;
    observer.stop().await;
    replacement.stop().await;
    server.stop().await;
}

#[tokio::test]
async fn websocket_rejects_invalid_tokens_and_requires_join_first() {
    let server = Server::start().await;
    for message in [
        ClientMsg::Ping,
        ClientMsg::Join {
            token: "garbage".into(),
        },
        ClientMsg::Join {
            token: token::sign(
                &MediaClaims {
                    exp: 0.0,
                    ..claims("alice", "room")
                },
                SECRET,
            ),
        },
        ClientMsg::Join {
            token: token::sign(&claims("alice", "room"), b"wrong"),
        },
    ] {
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{}/rtc", server.address))
            .await
            .unwrap();
        ws.send(Message::Text(
            serde_json::to_string(&message).unwrap().into(),
        ))
        .await
        .unwrap();
        let response: ServerMsg =
            serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert!(matches!(response, ServerMsg::Error { .. }));
        assert!(matches!(ws.next().await, Some(Ok(Message::Close(_)))));
    }
    assert_eq!(server.sfu.stats().participants, 0);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forwards_vp8_and_h264_simulcast_and_switches_subscriber_quality() {
    for codec in [Codec::Vp8, Codec::H264] {
        let server = Server::start().await;
        let mut a = Client::join_media(
            server.address,
            claims("alice", "room"),
            false,
            Some((TrackKind::Screen, codec)),
        )
        .await;
        a.ready().await;
        let mut b = Client::join(server.address, claims("bob", "room"), false).await;
        b.ready().await;
        let published = b.wait(|e| matches!(e, ClientEvent::Signal(ServerMsg::TrackPublished { track }) if track.kind == TrackKind::Screen)).await;
        let ClientEvent::Signal(ServerMsg::TrackPublished { track }) = published else {
            unreachable!()
        };
        let mid = track.mid;
        b.wait(|e| matches!(e, ClientEvent::Media { mid: received, data, .. } if received.to_string() == mid && data.last() == Some(&84))).await;
        b.control
            .send(Control::Signal(ClientMsg::SetVideoQuality {
                mid: mid.clone(),
                max_height: 180,
            }))
            .unwrap();
        b.wait(|e| matches!(e, ClientEvent::Media { mid: received, data, .. } if received.to_string() == mid && data.last() == Some(&42))).await;
        a.wait(|e| matches!(e, ClientEvent::Keyframe)).await;
        a.stop().await;
        b.stop().await;
        server.stop().await;
    }
}
