//! Single-port WebRTC SFU, embedded with [`Sfu::router`] or run as `pingo-sfu`.
//!
//! A single Tokio task owns all sans-IO str0m sessions. Signaling and backend hooks
//! use separate tasks so neither socket backpressure nor database I/O stalls media.

mod engine;
mod policy;
mod session;

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::routing::get;
use pingo_proto::{ClientMsg, ServerMsg};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};

use engine::{Command, Engine, Signal};

#[derive(Clone, Debug)]
pub struct SfuConfig {
    pub media_secret: Vec<u8>,
    pub udp_bind: SocketAddr,
    /// Advertised UDP host candidates. Empty selects local interface addresses.
    pub public_ips: Vec<IpAddr>,
    pub max_forwarded_audio: usize,
}

impl Default for SfuConfig {
    fn default() -> Self {
        Self {
            media_secret: Vec::new(),
            udp_bind: "0.0.0.0:7882".parse().unwrap(),
            public_ips: Vec::new(),
            max_forwarded_audio: 5,
        }
    }
}

/// Hooks run in delivery order on a separate task; implementations should finish
/// promptly. Keeping join/state/leave ordered prevents stale voice-state writes.
#[async_trait]
pub trait VoiceHooks: Send + Sync + 'static {
    async fn on_state(
        &self,
        room: &str,
        user: &str,
        muted: bool,
        deafened: bool,
        sharing_screen: bool,
        video: bool,
    );
    async fn on_leave(&self, room: &str, user: &str);
}

#[derive(Debug, thiserror::Error)]
pub enum SfuError {
    #[error("invalid SFU configuration: {0}")]
    Config(String),
    #[error("UDP socket: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SfuStats {
    pub rooms: usize,
    pub participants: usize,
    /// Authorized published source tracks, independent of subscriber count.
    pub tracks: usize,
    /// All accepted UDP datagrams (including ICE, DTLS and RTCP).
    pub bytes_in: u64,
    pub bytes_out: u64,
}

#[derive(Default)]
struct Counters {
    rooms: AtomicUsize,
    participants: AtomicUsize,
    tracks: AtomicUsize,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
}

#[derive(Clone)]
pub struct Sfu {
    commands: mpsc::Sender<Command>,
    counters: Arc<Counters>,
    udp_addr: SocketAddr,
}

impl Sfu {
    pub async fn start(cfg: SfuConfig, hooks: Arc<dyn VoiceHooks>) -> Result<Self, SfuError> {
        if cfg.media_secret.is_empty() {
            return Err(SfuError::Config("media_secret must not be empty".into()));
        }
        let socket = UdpSocket::bind(cfg.udp_bind).await?;
        let udp_addr = socket.local_addr()?;
        let ips = advertised_ips(&cfg, udp_addr)?;
        let (commands, rx) = mpsc::channel(512);
        let counters = Arc::new(Counters::default());
        let engine = Engine::new(cfg, socket, ips, counters.clone(), hooks);
        tokio::spawn(engine.run(rx));
        Ok(Self {
            commands,
            counters,
            udp_addr,
        })
    }

    pub fn router<S: Clone + Send + Sync + 'static>(&self) -> Router<S> {
        let sfu = self.clone();
        Router::new().route(
            "/rtc",
            get(move |ws: WebSocketUpgrade| {
                let sfu = sfu.clone();
                async move {
                    ws.max_message_size(128 * 1024)
                        .max_frame_size(128 * 1024)
                        .on_upgrade(move |socket| async move { sfu.websocket(socket).await })
                }
            }),
        )
    }

    pub fn stats(&self) -> SfuStats {
        SfuStats {
            rooms: self.counters.rooms.load(Ordering::Relaxed),
            participants: self.counters.participants.load(Ordering::Relaxed),
            tracks: self.counters.tracks.load(Ordering::Relaxed),
            bytes_in: self.counters.bytes_in.load(Ordering::Relaxed),
            bytes_out: self.counters.bytes_out.load(Ordering::Relaxed),
        }
    }

    /// Useful when binding port zero in embedded servers and localhost tests.
    pub fn udp_addr(&self) -> SocketAddr {
        self.udp_addr
    }

    /// Disconnects all participants and drains leave hooks before returning.
    pub async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        if self.commands.send(Command::Shutdown(tx)).await.is_ok() {
            let _ = rx.await;
        }
    }

    async fn websocket(&self, mut socket: WebSocket) {
        let first = tokio::time::timeout(Duration::from_secs(10), socket.recv()).await;
        let token = match first {
            Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str(&text) {
                Ok(ClientMsg::Join { token }) => token,
                _ => {
                    reject(&mut socket, "join_required", "first message must be Join").await;
                    return;
                }
            },
            _ => {
                reject(
                    &mut socket,
                    "join_required",
                    "Join timed out or was not JSON text",
                )
                .await;
                return;
            }
        };
        let (out, mut incoming) = mpsc::channel(128);
        let (reply, joined) = oneshot::channel();
        if self
            .commands
            .send(Command::Join { token, out, reply })
            .await
            .is_err()
        {
            return;
        }
        let Ok(Some(id)) = joined.await else {
            if let Some(Signal::Message(message)) = incoming.recv().await {
                let _ = send_signal(&mut socket, message).await;
            }
            let _ = socket.send(Message::Close(None)).await;
            return;
        };
        loop {
            tokio::select! {
                outgoing = incoming.recv() => {
                    match outgoing {
                        Some(Signal::Message(message)) => {
                            if send_signal(&mut socket, message).await.is_err() { break; }
                        }
                        Some(Signal::Close) | None => break,
                    }
                }
                frame = socket.recv() => {
                    let command = match frame {
                        Some(Ok(Message::Text(text))) => match serde_json::from_str(&text) {
                            Ok(message) => Command::Message { id: id.clone(), message },
                            Err(_) => {
                                reject(&mut socket, "bad_message", "invalid signaling JSON").await;
                                break;
                            }
                        },
                        Some(Ok(Message::Ping(_) | Message::Pong(_))) => Command::Touch(id.clone()),
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                        _ => continue,
                    };
                    // Congestion disconnects a signaling flooder instead of accumulating work.
                    if self.commands.try_send(command).is_err() { break; }
                }
            }
        }
        let _ = self.commands.send(Command::Leave(id)).await;
        let _ =
            tokio::time::timeout(Duration::from_secs(1), socket.send(Message::Close(None))).await;
    }
}

async fn send_signal(socket: &mut WebSocket, message: ServerMsg) -> Result<(), ()> {
    let json = serde_json::to_string(&message).map_err(|_| ())?;
    tokio::time::timeout(
        Duration::from_secs(2),
        socket.send(Message::Text(json.into())),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())
}

async fn reject(socket: &mut WebSocket, code: &str, message: &str) {
    let _ = send_signal(
        socket,
        ServerMsg::Error {
            code: code.into(),
            message: message.into(),
        },
    )
    .await;
    let _ = tokio::time::timeout(Duration::from_secs(1), socket.send(Message::Close(None))).await;
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as f64
}

fn advertised_ips(cfg: &SfuConfig, bound: SocketAddr) -> Result<Vec<IpAddr>, SfuError> {
    let mut ips = cfg.public_ips.clone();
    if ips.is_empty() {
        if !bound.ip().is_unspecified() {
            ips.push(bound.ip());
        } else {
            ips.extend(
                if_addrs::get_if_addrs()?
                    .into_iter()
                    .map(|i| i.ip())
                    .filter(|ip| {
                        !ip.is_loopback() && !ip.is_unspecified() && ip.is_ipv4() == bound.is_ipv4()
                    }),
            );
        }
    }
    ips.sort();
    ips.dedup();
    if ips.is_empty()
        || ips
            .iter()
            .any(|ip| ip.is_unspecified() || ip.is_multicast() || ip.is_ipv4() != bound.is_ipv4())
    {
        return Err(SfuError::Config(
            "provide public_ips matching the UDP socket address family".into(),
        ));
    }
    Ok(ips)
}
