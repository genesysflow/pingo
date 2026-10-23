use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use pingo_proto::{ClientMsg, DataMsg, MediaClaims, ServerMsg, TrackKind, token};
use str0m::bwe::BweKind;
use str0m::media::{KeyframeRequestKind, MediaData, Rid};
use str0m::net::{Protocol, Receive};
use str0m::{Event, IceConnectionState, Input, Output};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};

use crate::policy::{self, Level, Speakers};
use crate::session::{Peer, Source, TrackState, parse_mid};
use crate::{Counters, SfuConfig, VoiceHooks, now_ms};

pub(crate) enum Signal {
    Message(ServerMsg),
    Close,
}

pub(crate) enum Command {
    Join {
        token: String,
        media_secret: Option<Vec<u8>>,
        out: mpsc::Sender<Signal>,
        reply: oneshot::Sender<Option<String>>,
    },
    Message {
        id: String,
        message: ClientMsg,
    },
    Touch(String),
    Leave(String),
    Shutdown(oneshot::Sender<()>),
}

enum Hook {
    State {
        room: String,
        user: String,
        state: (bool, bool, bool, bool),
    },
    Leave {
        room: String,
        user: String,
    },
}

#[derive(Default)]
struct Room {
    peers: HashMap<String, Peer>,
    speakers: Speakers,
    speaking: HashSet<String>,
}

impl Room {
    fn broadcast(&mut self, message: ServerMsg, except: Option<&str>) {
        for peer in self
            .peers
            .values_mut()
            .filter(|p| Some(p.id.as_str()) != except)
        {
            peer.send(message.clone());
        }
    }

    fn reconcile(&mut self, now: Instant) {
        let sources: Vec<_> = self
            .peers
            .values()
            .flat_map(|p| p.sources.values().cloned())
            .collect();
        for peer in self.peers.values_mut() {
            peer.reconcile(&sources, now);
        }
    }

    fn keyframe(
        &mut self,
        source: &Source,
        rid: Option<Rid>,
        now: Instant,
        kind: KeyframeRequestKind,
    ) {
        if let Some(origin) = self.peers.get_mut(&source.origin) {
            origin.request_keyframe(source.mid, rid, now, kind);
        }
    }

    fn speaking(&mut self, n: usize, now: Instant) {
        let levels: Vec<_> = self
            .peers
            .values()
            .filter(|p| !p.muted && p.claims.can_speak)
            .filter_map(|p| p.level.map(|level| (p.id.clone(), level)))
            .collect();
        self.speakers.select(levels.clone().into_iter(), n, now);
        let mut active = HashSet::new();
        for (id, level) in levels.into_iter().filter(|(_, level)| level.active(now)) {
            active.insert(id.clone());
            self.data_all(&DataMsg::Speaking {
                participant_id: id,
                level: level.normalized(),
            });
        }
        let stopped: Vec<_> = self.speaking.difference(&active).cloned().collect();
        for id in stopped {
            self.data_all(&DataMsg::Speaking {
                participant_id: id,
                level: 0.0,
            });
        }
        self.speaking = active;
    }

    fn data_all(&mut self, message: &DataMsg) {
        if let Ok(json) = serde_json::to_vec(message) {
            for peer in self.peers.values_mut() {
                peer.write_data(&json);
            }
        }
    }

    fn relay(&mut self, id: &str, data: &[u8], now: Instant) {
        let Some(sender) = self.peers.get_mut(id) else {
            return;
        };
        if !sender.rate.allow(now) || data.len() > 16 * 1024 {
            return;
        }
        let Ok(message) = serde_json::from_slice::<DataMsg>(data) else {
            return;
        };
        let Some(message) = policy::stamp(message, id) else {
            return;
        };
        // Screen mids differ per session, so translate annotations to each receiver's mid.
        let share = match &message {
            DataMsg::Pointer { share_mid, .. } | DataMsg::Stroke { share_mid, .. } => {
                let Some(mid) = parse_mid(share_mid) else {
                    return;
                };
                sender
                    .sources
                    .get(&mid)
                    .filter(|s| s.kind == TrackKind::Screen)
                    .cloned()
                    .or_else(|| {
                        sender
                            .subscriptions
                            .iter()
                            .find(|s| {
                                s.mid() == Some(mid)
                                    && s.announced
                                    && !s.removed
                                    && s.source.kind == TrackKind::Screen
                            })
                            .map(|s| s.source.clone())
                    })
            }
            _ => None,
        };
        if matches!(message, DataMsg::Pointer { .. } | DataMsg::Stroke { .. }) && share.is_none() {
            return;
        }
        for peer in self.peers.values_mut().filter(|p| p.id != id) {
            let mut message = message.clone();
            if let Some(source) = &share {
                let mid = if peer.id == source.origin {
                    Some(source.mid)
                } else {
                    peer.subscriptions
                        .iter()
                        .find(|s| {
                            s.announced
                                && !s.removed
                                && s.source.origin == source.origin
                                && s.source.mid == source.mid
                        })
                        .and_then(|s| s.mid())
                };
                let Some(mid) = mid else {
                    continue;
                };
                match &mut message {
                    DataMsg::Pointer { share_mid, .. } | DataMsg::Stroke { share_mid, .. } => {
                        *share_mid = mid.to_string()
                    }
                    _ => {}
                }
            }
            if let Ok(json) = serde_json::to_vec(&message) {
                peer.write_data(&json);
            }
        }
    }

    fn forward(&mut self, id: &str, data: MediaData, max_audio: usize, now: Instant) {
        let Some(origin) = self.peers.get_mut(id) else {
            return;
        };
        let Some(source) = origin.sources.get(&data.mid).cloned() else {
            return;
        };
        let audio = policy::media_kind(source.kind).is_audio();
        if audio {
            if origin.muted {
                return;
            }
            // Peers without RFC 6464 still get a conservative equal-priority slot.
            origin.level = Some(Level {
                db: if data.ext_vals.voice_activity == Some(false) {
                    -127
                } else {
                    data.ext_vals.audio_level.unwrap_or(-40)
                },
                at: now,
            });
        } else if !data.contiguous {
            origin.request_keyframe(data.mid, data.rid, now, KeyframeRequestKind::Fir);
        }
        let eligible_audio = self
            .peers
            .values()
            .filter(|p| {
                p.claims.can_speak
                    && !p.muted
                    && p.sources
                        .values()
                        .any(|s| policy::media_kind(s.kind).is_audio())
            })
            .count();
        if audio
            && (max_audio == 0
                || (eligible_audio > max_audio && !self.speakers.selected.contains(id)))
        {
            return;
        }
        let mut requests = Vec::new();
        for peer in self
            .peers
            .values_mut()
            .filter(|p| p.id != id && (!audio || !p.deafened))
        {
            let video_count = peer
                .subscriptions
                .iter()
                .filter(|t| {
                    !t.removed && policy::media_kind(t.source.kind).is_video() && t.max_height > 0
                })
                .count()
                .max(1) as u64;
            // Reserve capacity for Opus, retransmits and estimation error.
            let budget = peer.bitrate.saturating_sub(128_000).saturating_mul(8) / 10 / video_count;
            let Some(track) = peer.subscriptions.iter_mut().find(|t| {
                !t.removed && t.announced && t.source.origin == id && t.source.mid == data.mid
            }) else {
                continue;
            };
            let TrackState::Open(mid) = track.state else {
                continue;
            };
            if !audio {
                if track.max_height == 0 {
                    track.started = false;
                    continue;
                }
                let wanted = policy::choose_layer(&track.source.layers, track.max_height, budget);
                let changed = !track.started || track.selected != wanted;
                if changed {
                    // Keep the old layer flowing until a keyframe arrives on the target layer.
                    requests.push((source.clone(), wanted));
                    if data.rid == wanted && data.is_keyframe() {
                        let upgrading = track
                            .source
                            .layers
                            .iter()
                            .position(|l| Some(l.rid) == wanted)
                            > track
                                .source
                                .layers
                                .iter()
                                .position(|l| Some(l.rid) == track.selected);
                        if !track.started
                            || !upgrading
                            || now.duration_since(track.last_switch) >= Duration::from_secs(1)
                        {
                            track.selected = wanted;
                            track.started = true;
                            track.last_switch = now;
                        }
                    }
                }
                if !track.started || data.rid != track.selected {
                    continue;
                }
            }
            let Some(mut writer) = peer.rtc.writer(mid) else {
                continue;
            };
            let Some(pt) = writer.match_params(data.params) else {
                continue;
            };
            // match_params uses the session codec map; this particular m-line
            // can have negotiated a smaller set than the rest of the session.
            if !writer.payload_params().any(|params| params.pt() == pt) {
                continue;
            }
            let time = if audio {
                data.time
            } else {
                let Some(time) = track.clock.map(data.rid, data.time, data.network_time) else {
                    continue;
                };
                time
            };
            if let Some(level) = data.ext_vals.audio_level {
                writer = writer.audio_level(level, data.ext_vals.voice_activity.unwrap_or(true));
            }
            if let Some(orientation) = data.ext_vals.video_orientation {
                writer = writer.video_orientation(orientation);
            }
            writer = writer.start_of_talkspurt(data.audio_start_of_talk_spurt);
            if let Err(error) = writer.write(pt, data.network_time, time, data.data.clone()) {
                tracing::debug!(participant = %peer.id, %error, "dropping media frame");
            }
        }
        for (source, rid) in requests {
            self.keyframe(&source, rid, now, KeyframeRequestKind::Pli);
        }
    }
}

pub(crate) struct Engine {
    cfg: SfuConfig,
    socket: UdpSocket,
    ips: Vec<IpAddr>,
    udp_routes: HashMap<SocketAddr, SocketAddr>,
    rooms: HashMap<String, Room>,
    locations: HashMap<String, String>,
    counters: Arc<Counters>,
    hooks: mpsc::UnboundedSender<Hook>,
    hook_task: tokio::task::JoinHandle<()>,
}

impl Engine {
    pub fn new(
        cfg: SfuConfig,
        socket: UdpSocket,
        ips: Vec<IpAddr>,
        counters: Arc<Counters>,
        hooks: Arc<dyn VoiceHooks>,
    ) -> Self {
        let (hook_tx, mut rx) = mpsc::unbounded_channel();
        let hook_task = tokio::spawn(async move {
            while let Some(hook) = rx.recv().await {
                let delivery = async {
                    match hook {
                        Hook::State {
                            room,
                            user,
                            state: (muted, deafened, sharing, video),
                        } => {
                            hooks
                                .on_state(&room, &user, muted, deafened, sharing, video)
                                .await
                        }
                        Hook::Leave { room, user } => hooks.on_leave(&room, &user).await,
                    }
                };
                if tokio::time::timeout(Duration::from_secs(5), delivery)
                    .await
                    .is_err()
                {
                    tracing::warn!("voice hook timed out");
                }
            }
        });
        Self {
            cfg,
            socket,
            ips,
            udp_routes: HashMap::new(),
            rooms: HashMap::new(),
            locations: HashMap::new(),
            counters,
            hooks: hook_tx,
            hook_task,
        }
    }

    pub async fn run(mut self, mut commands: mpsc::Receiver<Command>) {
        let mut buffer = vec![0; 65536];
        let mut speaking_at = Instant::now();
        let mut shutdown = None;
        loop {
            let now = Instant::now();
            let expired: Vec<_> = self
                .rooms
                .values()
                .flat_map(|r| r.peers.values())
                .filter(|p| p.expired(now))
                .map(|p| p.id.clone())
                .collect();
            for id in expired {
                self.leave(&id);
            }
            if now >= speaking_at {
                for room in self.rooms.values_mut() {
                    room.speaking(self.cfg.max_forwarded_audio, now);
                }
                speaking_at = now + Duration::from_millis(200);
            }
            let mut next = speaking_at;
            let mut events = Vec::new();
            for (room_id, room) in &mut self.rooms {
                for peer in room.peers.values_mut() {
                    peer.negotiate(now);
                    if let Err(error) = peer.rtc.handle_input(Input::Timeout(now)) {
                        tracing::debug!(%error, "RTC timeout failed");
                        peer.dead = true;
                        continue;
                    }
                    // A busy session must yield so commands and other rooms make progress.
                    for _ in 0..256 {
                        match peer.rtc.poll_output() {
                            Ok(Output::Timeout(at)) => {
                                next = next.min(at);
                                break;
                            }
                            Ok(Output::Transmit(tx)) => {
                                match self.socket.try_send_to(&tx.contents, tx.destination) {
                                    Ok(n) => {
                                        self.counters
                                            .bytes_out
                                            .fetch_add(n as u64, Ordering::Relaxed);
                                    }
                                    Err(error)
                                        if error.kind() == std::io::ErrorKind::WouldBlock => {}
                                    Err(error) => tracing::debug!(%error, "UDP send failed"),
                                }
                            }
                            Ok(Output::Event(event)) => {
                                events.push((room_id.clone(), peer.id.clone(), event))
                            }
                            Err(error) => {
                                tracing::debug!(participant = %peer.id, %error, "RTC output failed");
                                peer.dead = true;
                                break;
                            }
                        }
                    }
                }
            }
            for (room_id, id, event) in events {
                self.event(&room_id, &id, event, now);
            }
            self.update_stats();
            // Polling generated output can make more output ready in another session.
            next = next.min(Instant::now() + Duration::from_millis(10));
            tokio::select! {
                command = commands.recv() => match command {
                    Some(Command::Shutdown(reply)) => { shutdown = Some(reply); break; }
                    Some(command) => self.command(command),
                    None => break,
                },
                received = self.socket.recv_from(&mut buffer) => {
                    if let Ok((len, source)) = received {
                        let Ok(contents) = buffer[..len].try_into() else { continue; };
                        let bound = self.socket.local_addr().expect("bound socket");
                        let mut input = Input::Receive(Instant::now(), Receive { proto: Protocol::Udp, source, destination: bound, contents });
                        if let Some(peer) = self.rooms.values_mut().flat_map(|r| r.peers.values_mut()).find(|p| p.rtc.accepts(&input)) {
                            // recv_from on a wildcard socket reports no destination IP.
                            // ICE must see an advertised candidate, never 0.0.0.0/::.
                            // Resolve only authenticated/demultiplexed peers and bound the cache.
                            if bound.ip().is_unspecified() {
                                let destination = self.udp_routes.get(&source).copied().unwrap_or_else(|| {
                                    let destination = udp_destination(bound, source, &self.ips);
                                    if self.udp_routes.len() < 1024 { self.udp_routes.insert(source, destination); }
                                    destination
                                });
                                if let Input::Receive(_, receive) = &mut input { receive.destination = destination; }
                            }
                            self.counters.bytes_in.fetch_add(len as u64, Ordering::Relaxed);
                            // ICE keepalives count as activity, as do encrypted media/data packets.
                            if peer.rtc.handle_input(input).is_ok() { peer.last_seen = Instant::now(); } else { peer.dead = true; }
                        }
                    }
                }
                _ = tokio::time::sleep_until(next.into()) => {}
            }
        }
        let ids: Vec<_> = self.locations.keys().cloned().collect();
        for id in ids {
            self.leave(&id);
        }
        self.update_stats();
        drop(self.hooks);
        let _ = self.hook_task.await;
        if let Some(reply) = shutdown {
            let _ = reply.send(());
        }
    }

    fn command(&mut self, command: Command) {
        let now = Instant::now();
        match command {
            Command::Join {
                token,
                media_secret,
                out,
                reply,
            } => {
                let claims = match verify_join(
                    &token,
                    media_secret.as_deref().unwrap_or(&self.cfg.media_secret),
                ) {
                    Ok(claims) => claims,
                    Err(message) => {
                        let _ = out.try_send(Signal::Message(ServerMsg::Error {
                            code: "invalid_token".into(),
                            message,
                        }));
                        let _ = reply.send(None);
                        return;
                    }
                };
                let duplicate: Vec<_> = self
                    .rooms
                    .values()
                    .flat_map(|r| r.peers.values())
                    .filter(|p| p.claims.user == claims.user)
                    .map(|p| p.id.clone())
                    .collect();
                for id in duplicate {
                    if let Some(peer) = self.peer_mut(&id) {
                        peer.close("replaced", "another session joined for this user");
                    }
                    self.leave(&id);
                }
                let Ok(mut peer) = Peer::new(
                    claims,
                    out,
                    self.socket.local_addr().expect("bound socket"),
                    &self.ips,
                    now,
                ) else {
                    let _ = reply.send(None);
                    return;
                };
                let id = peer.id.clone();
                let room_id = peer.claims.room.clone();
                let room = self.rooms.entry(room_id.clone()).or_default();
                let mut participants: Vec<_> = room.peers.values().map(Peer::info).collect();
                participants.sort_by(|a, b| a.participant_id.cmp(&b.participant_id));
                peer.send(ServerMsg::Joined {
                    participant_id: id.clone(),
                    participants,
                });
                room.broadcast(
                    ServerMsg::ParticipantJoined {
                        participant: peer.info(),
                    },
                    None,
                );
                let _ = self.hooks.send(Hook::State {
                    room: room_id.clone(),
                    user: peer.claims.user.clone(),
                    state: peer.current_hook_state(),
                });
                room.peers.insert(id.clone(), peer);
                self.locations.insert(id.clone(), room_id);
                room.reconcile(now);
                if reply.send(Some(id.clone())).is_err() {
                    self.leave(&id);
                }
            }
            Command::Touch(id) => {
                if let Some(peer) = self.peer_mut(&id) {
                    peer.last_seen = now;
                }
            }
            Command::Leave(id) => self.leave(&id),
            Command::Message { id, message } => {
                let Some(room_id) = self.locations.get(&id).cloned() else {
                    return;
                };
                let room = self.rooms.get_mut(&room_id).unwrap();
                let peer = room.peers.get_mut(&id).unwrap();
                if !peer.signal_rate.allow(now) {
                    peer.close("rate_limited", "too many signaling messages");
                    return;
                }
                peer.last_seen = now;
                let result = match message {
                    ClientMsg::Join { .. } => Err("already joined".into()),
                    ClientMsg::Ping => {
                        peer.send(ServerMsg::Pong);
                        Ok(Vec::new())
                    }
                    ClientMsg::Offer { sdp } => peer.offer(&sdp, now).map(|_| Vec::new()),
                    ClientMsg::Answer { sdp } => peer.answer(&sdp),
                    ClientMsg::PublishTracks { tracks } => peer.declare(tracks).map(|_| Vec::new()),
                    ClientMsg::SetState { muted, deafened } => {
                        let muted = muted || !peer.claims.can_speak;
                        if (peer.muted, peer.deafened) != (muted, deafened) {
                            peer.muted = muted;
                            peer.deafened = deafened;
                            if muted {
                                peer.level = None;
                            }
                            let info = peer.info();
                            room.broadcast(
                                ServerMsg::ParticipantUpdated { participant: info },
                                None,
                            );
                        }
                        Ok(Vec::new())
                    }
                    ClientMsg::SetVideoQuality { mid, max_height } => {
                        if let Some(mid) = parse_mid(&mid)
                            && let Some(track) = peer.subscriptions.iter_mut().find(|t| {
                                t.mid() == Some(mid)
                                    && !t.removed
                                    && policy::media_kind(t.source.kind).is_video()
                            })
                        {
                            track.max_height = max_height.min(4320);
                        }
                        Ok(Vec::new())
                    }
                };
                match result {
                    Ok(requests) => {
                        for (source, rid) in requests {
                            room.keyframe(&source, rid, now, KeyframeRequestKind::Pli);
                        }
                    }
                    Err(error) => room
                        .peers
                        .get_mut(&id)
                        .unwrap()
                        .close("bad_signaling", &error),
                }
                room.reconcile(now);
                self.notify_state(&room_id, &id);
            }
            Command::Shutdown(_) => unreachable!(),
        }
    }

    fn event(&mut self, room_id: &str, id: &str, event: Event, now: Instant) {
        let Some(room) = self.rooms.get_mut(room_id) else {
            return;
        };
        let Some(peer) = room.peers.get_mut(id) else {
            return;
        };
        match event {
            Event::Connected => peer.connected = true,
            Event::IceConnectionStateChange(IceConnectionState::Disconnected) => peer.dead = true,
            Event::MediaAdded(media) => {
                if media.direction.is_receiving()
                    && let Some(simulcast) = media.simulcast
                {
                    peer.simulcast.insert(media.mid, simulcast.recv);
                }
                peer.refresh_sources();
                room.reconcile(now);
                self.notify_state(room_id, id);
            }
            Event::MediaChanged(_) => {
                peer.refresh_sources();
                room.reconcile(now);
                self.notify_state(room_id, id);
            }
            Event::MediaData(data) => room.forward(id, data, self.cfg.max_forwarded_audio, now),
            Event::ChannelOpen(cid, label) if label == "pingo" && peer.cid.is_none() => {
                peer.cid = Some(cid)
            }
            Event::ChannelClose(cid) if peer.cid == Some(cid) => peer.cid = None,
            Event::ChannelData(data) if peer.cid == Some(data.id) => {
                room.relay(id, &data.data, now)
            }
            Event::EgressBitrateEstimate(
                BweKind::Twcc { estimate, .. } | BweKind::Remb { estimate, .. },
            ) => peer.bitrate = estimate.as_u64(),
            Event::KeyframeRequest(request) => {
                if let Some(track) = peer
                    .subscriptions
                    .iter()
                    .find(|t| t.mid() == Some(request.mid) && !t.removed)
                {
                    let (source, rid) = (track.source.clone(), track.selected);
                    room.keyframe(&source, rid, now, request.kind);
                }
            }
            _ => {}
        }
    }

    fn peer_mut(&mut self, id: &str) -> Option<&mut Peer> {
        self.rooms
            .get_mut(self.locations.get(id)?)?
            .peers
            .get_mut(id)
    }

    fn notify_state(&mut self, room_id: &str, id: &str) {
        if let Some(peer) = self
            .rooms
            .get_mut(room_id)
            .and_then(|r| r.peers.get_mut(id))
        {
            let state = peer.current_hook_state();
            if state != peer.hook_state {
                peer.hook_state = state;
                let _ = self.hooks.send(Hook::State {
                    room: room_id.into(),
                    user: peer.claims.user.clone(),
                    state,
                });
            }
        }
    }

    fn leave(&mut self, id: &str) {
        let Some(room_id) = self.locations.remove(id) else {
            return;
        };
        let room = self.rooms.get_mut(&room_id).unwrap();
        let Some(mut peer) = room.peers.remove(id) else {
            return;
        };
        peer.rtc.disconnect();
        peer.close_socket();
        room.broadcast(
            ServerMsg::ParticipantLeft {
                participant_id: id.into(),
            },
            None,
        );
        room.reconcile(Instant::now());
        let _ = self.hooks.send(Hook::Leave {
            room: room_id.clone(),
            user: peer.claims.user,
        });
        if room.peers.is_empty() {
            self.rooms.remove(&room_id);
        }
    }

    fn update_stats(&self) {
        self.counters
            .rooms
            .store(self.rooms.len(), Ordering::Relaxed);
        self.counters
            .participants
            .store(self.locations.len(), Ordering::Relaxed);
        self.counters.tracks.store(
            self.rooms
                .values()
                .flat_map(|r| r.peers.values())
                .map(|p| p.sources.len())
                .sum(),
            Ordering::Relaxed,
        );
    }
}

fn verify_join(value: &str, secret: &[u8]) -> Result<MediaClaims, String> {
    let claims = token::verify(value, secret, now_ms()).map_err(|e| e.to_string())?;
    if claims.room.is_empty() || claims.user.is_empty() || !claims.exp.is_finite() {
        return Err("invalid media claims".into());
    }
    Ok(claims)
}

// Prefer the local route when advertised. Behind NAT the public candidate is
// a logical address on this same socket, so use it when the route is private.
fn udp_destination(bound: SocketAddr, source: SocketAddr, ips: &[IpAddr]) -> SocketAddr {
    let route = (|| -> std::io::Result<IpAddr> {
        let probe = std::net::UdpSocket::bind(if source.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        })?;
        probe.connect(source)?;
        Ok(probe.local_addr()?.ip())
    })()
    .ok();
    let ip = route.filter(|ip| ips.contains(ip)).unwrap_or(ips[0]);
    SocketAddr::new(ip, bound.port())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct NoHooks;
    #[async_trait::async_trait]
    impl VoiceHooks for NoHooks {
        async fn on_state(&self, _: &str, _: &str, _: bool, _: bool, _: bool, _: bool) {}
        async fn on_leave(&self, _: &str, _: &str) {}
    }

    #[tokio::test]
    async fn room_bookkeeping_replacement_and_late_disconnect_are_isolated() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut engine = Engine::new(
            SfuConfig {
                media_secret: b"secret".to_vec(),
                ..Default::default()
            },
            socket,
            vec!["127.0.0.1".parse().unwrap()],
            Arc::new(Counters::default()),
            Arc::new(NoHooks),
        );
        let mut retained = Vec::new();
        let mut ids = Vec::new();
        for (room, user) in [
            ("one", "alice"),
            ("one", "bob"),
            ("two", "carol"),
            ("two", "alice"),
        ] {
            let claims = MediaClaims {
                room: room.into(),
                user: user.into(),
                name: user.into(),
                color: "red".into(),
                can_speak: true,
                can_share: false,
                can_video: false,
                exp: now_ms() + 60_000.0,
            };
            let (out, rx) = mpsc::channel(128);
            retained.push(rx);
            let (reply, response) = oneshot::channel();
            engine.command(Command::Join {
                token: token::sign(&claims, b"secret"),
                media_secret: None,
                out,
                reply,
            });
            ids.push(response.await.unwrap().unwrap());
        }
        assert_eq!(engine.rooms.len(), 2);
        assert_eq!(engine.locations.len(), 3);
        assert_eq!(engine.rooms["one"].peers.len(), 1);
        assert_eq!(engine.rooms["two"].peers.len(), 2);
        engine.leave(&ids[0]); // The old websocket closes after the replacement joins.
        assert_eq!(engine.locations.len(), 3);
        engine.leave(&ids[1]);
        assert!(!engine.rooms.contains_key("one"));
        engine.leave(&ids[2]);
        engine.leave(&ids[3]);
        assert!(engine.rooms.is_empty());
        assert!(engine.locations.is_empty());
        engine.update_stats();
        assert_eq!(engine.counters.participants.load(Ordering::Relaxed), 0);
        drop(engine.hooks);
        engine.hook_task.await.unwrap();
    }

    #[test]
    fn rejects_bad_expired_and_wrong_secret_tokens() {
        assert!(verify_join("bad", b"secret").is_err());
        let mut claims = MediaClaims {
            room: "r".into(),
            user: "u".into(),
            name: "Ann".into(),
            color: "red".into(),
            can_speak: true,
            can_video: false,
            can_share: false,
            exp: now_ms() - 1000.0,
        };
        assert!(verify_join(&token::sign(&claims, b"secret"), b"secret").is_err());
        claims.exp = now_ms() + 60_000.0;
        let valid = token::sign(&claims, b"secret");
        assert!(verify_join(&valid, b"other").is_err());
        assert_eq!(verify_join(&valid, b"secret").unwrap(), claims);
    }
}
