//! A real TURN allocation carries ICE, DTLS, Opus and SCTP without audio hardware.
use pingo_media::{
    audio::{denoise::NoiseLevel, level::Gate},
    pipeline::AudioSettings,
    rtc::{Client, ClientConfig, Command, RtcEvent},
};
use pingo_media_test_server::Server;
use pingo_proto::{
    DataMsg,
    ice::{IceServer, turn_password},
};
use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::UdpSocket,
    sync::{broadcast, mpsc},
};
use turn::{
    auth::{AuthHandler, generate_auth_key},
    relay::relay_static::RelayAddressGeneratorStatic,
    server::{
        Server as TurnServer,
        config::{ConnConfig, ServerConfig},
    },
};
use webrtc_util::vnet::net::Net;

struct Auth;
impl AuthHandler for Auth {
    fn auth_handle(
        &self,
        username: &str,
        realm: &str,
        _: SocketAddr,
    ) -> Result<Vec<u8>, turn::Error> {
        if username != "pingo-test" {
            return Err(turn::Error::ErrFakeErr);
        }
        Ok(generate_auth_key(username, realm, "test-only-password"))
    }
}
async fn exercise(servers: Vec<IceServer>, relay_only: bool) {
    let backend = Server::start().await;
    let tokens = backend.tokens().await;
    let mut commands = vec![];
    let mut events = vec![];
    let mut workers = vec![];
    for token in tokens {
        let mut config = ClientConfig::new(backend.url.clone(), token);
        config.ice_servers = servers.clone();
        config.relay_only = relay_only;
        config.settings = AudioSettings {
            noise_level: NoiseLevel::Off,
            echo_cancel: false,
            agc: false,
            gate: Gate::Open,
            ..Default::default()
        };
        let client = Client::connect(config).await.unwrap();
        events.push(client.events.subscribe());
        let (tx, rx) = mpsc::channel(64);
        commands.push(tx);
        workers.push(tokio::task::spawn_local(client.run(None, rx)));
    }
    for receiver in &mut events {
        tokio::time::timeout(Duration::from_secs(12), async {
            let mut relay_sdp = false;
            loop {
                match receiver.recv().await.unwrap() {
                    RtcEvent::Negotiated { local_sdp, .. } => {
                        if relay_only {
                            assert!(local_sdp.contains(" typ relay"));
                            assert!(!local_sdp.contains(" typ host"));
                        } else {
                            assert!(local_sdp.contains(" typ host"));
                        }
                        relay_sdp = true;
                    }
                    RtcEvent::Connected => {
                        assert!(relay_sdp);
                        break;
                    }
                    RtcEvent::Error { code, message } => panic!("{code}: {message}"),
                    _ => {}
                }
            }
        })
        .await
        .expect("relay-only ICE must connect");
    }
    // Emit on a worker clock while receiving; no capture device is involved.
    let sender = commands[0].clone();
    let capture = tokio::task::spawn_local(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(10));
        for step in 0..500 {
            tick.tick().await;
            let frame = std::array::from_fn(|i| {
                let sample = ((step * 480 + i) as f32 * 440.0 * std::f32::consts::TAU / 48000.0)
                    .sin()
                    * 0.15;
                [sample, sample]
            });
            if sender
                .send(Command::Capture(Box::new(frame)))
                .await
                .is_err()
            {
                break;
            }
            let _ = sender
                .send(Command::Data(DataMsg::RaiseHand {
                    participant_id: String::new(),
                    raised: true,
                }))
                .await;
        }
    });
    tokio::time::timeout(Duration::from_secs(8), async {
        let (mut audio, mut data) = (false, false);
        while !(audio && data) {
            match events[1].recv().await {
                Ok(RtcEvent::PlayoutStats(stats)) => {
                    audio |= stats.values().any(|s| s.received > 10)
                }
                Ok(RtcEvent::Data(DataMsg::RaiseHand { raised: true, .. })) => data = true,
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(error) => panic!("{error}"),
                _ => {}
            }
        }
    })
    .await
    .expect("Opus and SCTP must traverse TURN");
    capture.abort();
    let _ = capture.await;
    for tx in commands {
        let _ = tx.send(Command::Leave).await;
    }
    for task in workers {
        task.await.unwrap().unwrap();
    }
    backend.stop().await;
}

#[tokio::test]
async fn relay_only_native_audio_and_data() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let address = socket.local_addr().unwrap();
            let (closed_tx, mut closed) = mpsc::channel(8);
            let server = TurnServer::new(ServerConfig {
                conn_configs: vec![ConnConfig {
                    conn: Arc::new(socket),
                    relay_addr_generator: Box::new(RelayAddressGeneratorStatic {
                        relay_address: "127.0.0.1".parse().unwrap(),
                        address: "127.0.0.1".into(),
                        net: Arc::new(Net::new(None)),
                    }),
                }],
                realm: "pingo.test".into(),
                auth_handler: Arc::new(Auth),
                channel_bind_timeout: Duration::ZERO,
                alloc_close_notify: Some(closed_tx),
            })
            .await
            .unwrap();
            exercise(
                vec![IceServer {
                    urls: vec![format!("turn:{address}")],
                    username: "pingo-test".into(),
                    credential: "test-only-password".into(),
                }],
                true,
            )
            .await;
            tokio::time::timeout(Duration::from_secs(3), async {
                closed.recv().await.expect("first allocation released");
                closed.recv().await.expect("second allocation released");
            })
            .await
            .expect("leaving must release TURN allocations");
            server.close().await.unwrap();
        })
        .await;
}

#[tokio::test]
#[ignore = "requires a local coturn; see deploy/test-turn.sh"]
async fn coturn_native_audio_and_data() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let url = std::env::var("PINGO_TEST_TURN_URL").expect("PINGO_TEST_TURN_URL");
            let secret = std::env::var("PINGO_TEST_TURN_SECRET").expect("PINGO_TEST_TURN_SECRET");
            let expires = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600;
            let username = format!("{expires}:native-test");
            let credential = turn_password(&username, secret.as_bytes());
            exercise(
                vec![IceServer {
                    urls: vec![url],
                    username,
                    credential,
                }],
                true,
            )
            .await;
        })
        .await;
}

#[tokio::test]
#[ignore = "requires a local coturn; see deploy/test-turn.sh"]
async fn coturn_rejects_bad_credentials() {
    let url = std::env::var("PINGO_TEST_TURN_URL").unwrap();
    let secret = std::env::var("PINGO_TEST_TURN_SECRET").unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let expired = std::env::var_os("PINGO_TEST_TURN_EXPIRED").is_some();
    let expires = if expired { now - 10 } else { now + 3600 };
    let username = format!("{expires}:negative-test");
    let credential = if expired || std::env::var_os("PINGO_TEST_TURN_UNTRUSTED").is_some() {
        turn_password(&username, secret.as_bytes())
    } else {
        "incorrect-password".into()
    };
    let mut config = ClientConfig::new("ws://127.0.0.1:1/rtc".into(), "unused".into());
    config.relay_only = true;
    config.ice_servers = vec![IceServer {
        urls: vec![url],
        username,
        credential,
    }];
    match Client::connect(config).await {
        Err(pingo_media::Error::Invalid("no TURN relay available")) => {}
        _ => panic!(
            "invalid credentials or an untrusted TLS certificate must fail allocation before signaling"
        ),
    }
}

#[tokio::test]
async fn unavailable_turn_preserves_direct_media() {
    tokio::task::LocalSet::new()
        .run_until(async {
            exercise(
                vec![IceServer {
                    urls: vec!["turn:127.0.0.1:1?transport=tcp".into()],
                    username: "test".into(),
                    credential: "test".into(),
                }],
                false,
            )
            .await;
        })
        .await;
}
