use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pingo_proto::signal::PublishedTrack;
use pingo_proto::{MediaClaims, ParticipantInfo, ServerMsg, TrackInfo, TrackKind};
use str0m::bwe::Bitrate;
use str0m::change::{SdpAnswer, SdpOffer, SdpPendingOffer};
use str0m::channel::ChannelId;
use str0m::media::{Direction, KeyframeRequestKind, Mid, Rid, SimulcastLayer};
use str0m::{Candidate, Rtc};
use tokio::sync::mpsc;

use crate::engine::Signal;
use crate::policy::{self, FrameClock, Layer, Level, RateLimit};

#[derive(Clone)]
pub(crate) struct Source {
    pub origin: String,
    pub mid: Mid,
    pub kind: TrackKind,
    pub layers: Vec<Layer>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrackState {
    New,
    Adding(Mid),
    Open(Mid),
    Stop(Mid),
    Stopping(Mid),
}

pub(crate) struct Subscription {
    pub source: Source,
    pub state: TrackState,
    pub removed: bool,
    pub announced: bool,
    pub max_height: u32,
    pub selected: Option<Rid>,
    pub started: bool,
    pub last_switch: Instant,
    pub clock: FrameClock,
}

impl Subscription {
    pub fn mid(&self) -> Option<Mid> {
        match self.state {
            TrackState::New => None,
            TrackState::Adding(mid)
            | TrackState::Open(mid)
            | TrackState::Stop(mid)
            | TrackState::Stopping(mid) => Some(mid),
        }
    }
}

pub(crate) struct Peer {
    pub id: String,
    pub claims: MediaClaims,
    pub rtc: Rtc,
    out: mpsc::Sender<Signal>,
    pub last_seen: Instant,
    pub dead: bool,
    pub muted: bool,
    pub deafened: bool,
    pub connected: bool,
    pub initialized: bool,
    pub cid: Option<ChannelId>,
    pub pending: Option<SdpPendingOffer>,
    pub pending_since: Instant,
    pub negotiate_after: Instant,
    pub declarations: HashMap<Mid, TrackKind>,
    pub sources: HashMap<Mid, Source>,
    pub simulcast: HashMap<Mid, Vec<SimulcastLayer>>,
    pub subscriptions: Vec<Subscription>,
    pub level: Option<Level>,
    pub rate: RateLimit,
    pub signal_rate: RateLimit,
    pub bitrate: u64,
    keyframes: HashMap<(Mid, Option<Rid>), Instant>,
    pub hook_state: (bool, bool, bool, bool),
}

impl Peer {
    pub fn new(
        claims: MediaClaims,
        out: mpsc::Sender<Signal>,
        addr: SocketAddr,
        ips: &[IpAddr],
        now: Instant,
    ) -> Result<Self, str0m::RtcError> {
        let mut rtc = Rtc::builder()
            .set_crypto_provider(Arc::new(str0m::crypto::from_feature_flags()))
            .clear_codecs()
            .enable_opus(true, true)
            .enable_vp8(true)
            .enable_h264(true)
            .enable_bwe(Some(Bitrate::mbps(2)))
            .set_sctp_max_buffered_amount(256 * 1024)
            .build(now);
        for ip in ips {
            rtc.add_local_candidate(Candidate::host(SocketAddr::new(*ip, addr.port()), "udp")?);
        }
        let muted = !claims.can_speak;
        Ok(Self {
            id: uuid::Uuid::new_v4().simple().to_string(),
            claims,
            rtc,
            out,
            last_seen: now,
            dead: false,
            muted,
            deafened: false,
            connected: false,
            initialized: false,
            cid: None,
            pending: None,
            pending_since: now,
            negotiate_after: now,
            declarations: HashMap::new(),
            sources: HashMap::new(),
            simulcast: HashMap::new(),
            subscriptions: Vec::new(),
            level: None,
            rate: RateLimit::new(now),
            signal_rate: RateLimit::new(now),
            bitrate: 2_000_000,
            keyframes: HashMap::new(),
            hook_state: (muted, false, false, false),
        })
    }

    pub fn info(&self) -> ParticipantInfo {
        ParticipantInfo {
            participant_id: self.id.clone(),
            user: self.claims.user.clone(),
            name: self.claims.name.clone(),
            color: self.claims.color.clone(),
            muted: self.muted,
            deafened: self.deafened,
        }
    }

    pub fn send(&mut self, message: ServerMsg) {
        if self.out.try_send(Signal::Message(message)).is_err() {
            self.dead = true;
        }
    }

    pub fn out_closed(&self) -> bool {
        self.out.is_closed()
    }

    pub fn expired(&self, now: Instant) -> bool {
        self.dead
            || self.out_closed()
            || !self.rtc.is_alive()
            || now.duration_since(self.last_seen) > Duration::from_secs(30)
            || (self.pending.is_some()
                && now.duration_since(self.pending_since) > Duration::from_secs(10))
    }

    pub fn close_socket(&self) {
        let _ = self.out.try_send(Signal::Close);
    }

    pub fn close(&mut self, code: &str, message: &str) {
        self.send(ServerMsg::Error {
            code: code.into(),
            message: message.into(),
        });
        let _ = self.out.try_send(Signal::Close);
        self.dead = true;
    }

    pub fn offer(&mut self, sdp: &str, now: Instant) -> Result<(), String> {
        let offer = SdpOffer::from_sdp_string(sdp).map_err(|e| e.to_string())?;
        let answer = self
            .rtc
            .sdp_api()
            .accept_offer(offer)
            .map_err(|e| e.to_string())?;
        // str0m rolls pending changes back when accepting the winning client offer.
        self.pending = None;
        for track in &mut self.subscriptions {
            track.state = match track.state {
                TrackState::Adding(_) => TrackState::New,
                TrackState::Stopping(mid) => TrackState::Stop(mid),
                state => state,
            };
        }
        self.initialized = true;
        self.negotiate_after = now + Duration::from_millis(50);
        self.send(ServerMsg::Answer {
            sdp: opus_sdp(answer.to_sdp_string()),
        });
        self.refresh_sources();
        Ok(())
    }

    pub fn answer(&mut self, sdp: &str) -> Result<Vec<(Source, Option<Rid>)>, String> {
        let Some(pending) = self.pending.take() else {
            // A late answer for a rolled-back offer is harmless.
            return Ok(Vec::new());
        };
        let answer = SdpAnswer::from_sdp_string(sdp).map_err(|e| e.to_string())?;
        self.rtc
            .sdp_api()
            .accept_answer(pending, answer)
            .map_err(|e| e.to_string())?;
        let mut messages = Vec::new();
        let mut keyframes = Vec::new();
        for track in &mut self.subscriptions {
            if let TrackState::Adding(mid) = track.state {
                track.state = if track.removed {
                    TrackState::Stop(mid)
                } else {
                    TrackState::Open(mid)
                };
                if !track.removed
                    && self
                        .rtc
                        .media(mid)
                        .is_some_and(|m| !m.disabled() && m.direction().is_sending())
                {
                    track.announced = true;
                    messages.push(ServerMsg::TrackPublished {
                        track: TrackInfo {
                            participant_id: track.source.origin.clone(),
                            mid: mid.to_string(),
                            kind: track.source.kind,
                        },
                    });
                    if policy::media_kind(track.source.kind).is_video() {
                        keyframes.push((track.source.clone(), track.selected));
                    }
                }
            }
        }
        self.subscriptions
            .retain(|t| !matches!(t.state, TrackState::Stopping(_)));
        for message in messages {
            self.send(message);
        }
        Ok(keyframes)
    }

    pub fn declare(&mut self, tracks: Vec<PublishedTrack>) -> Result<(), String> {
        if !self.initialized || tracks.len() > 32 {
            return Err("PublishTracks requires an offer and at most 32 tracks".into());
        }
        let mut declarations = HashMap::new();
        for track in tracks {
            let mid = parse_mid(&track.mid).ok_or("invalid media id")?;
            let media = self.rtc.media(mid).ok_or("unknown media id")?;
            if !media.direction().is_receiving()
                || media.kind() != policy::media_kind(track.kind)
                || declarations.insert(mid, track.kind).is_some()
            {
                return Err("track must name a unique sending mid of the declared kind".into());
            }
        }
        self.declarations = declarations;
        self.refresh_sources();
        Ok(())
    }

    pub fn refresh_sources(&mut self) {
        self.sources.retain(|mid, source| {
            self.declarations.get(mid) == Some(&source.kind)
                && self.rtc.media(*mid).is_some_and(|m| {
                    !m.disabled()
                        && m.direction().is_receiving()
                        && m.kind() == policy::media_kind(source.kind)
                })
        });
        for (&mid, &kind) in &self.declarations {
            if !policy::allowed(&self.claims, kind)
                || !self.rtc.media(mid).is_some_and(|m| {
                    !m.disabled()
                        && m.direction().is_receiving()
                        && m.kind() == policy::media_kind(kind)
                })
            {
                continue;
            }
            let layers = policy::layers(
                self.simulcast.get(&mid).map_or(&[], Vec::as_slice),
                kind == TrackKind::Screen,
            );
            self.sources
                .entry(mid)
                .and_modify(|s| s.layers = layers.clone())
                .or_insert(Source {
                    origin: self.id.clone(),
                    mid,
                    kind,
                    layers,
                });
        }
    }

    pub fn reconcile(&mut self, sources: &[Source], now: Instant) {
        let mut messages = Vec::new();
        for track in &mut self.subscriptions {
            if !sources.iter().any(|s| {
                s.origin == track.source.origin
                    && s.mid == track.source.mid
                    && s.kind == track.source.kind
            }) {
                track.removed = true;
                if let TrackState::Open(mid) = track.state {
                    track.state = TrackState::Stop(mid);
                }
                if track.announced {
                    track.announced = false;
                    messages.push(ServerMsg::TrackUnpublished {
                        participant_id: track.source.origin.clone(),
                        mid: track.mid().unwrap().to_string(),
                    });
                }
            }
        }
        self.subscriptions
            .retain(|t| !(t.removed && t.state == TrackState::New));
        for source in sources.iter().filter(|s| s.origin != self.id) {
            if let Some(existing) = self.subscriptions.iter_mut().find(|t| {
                !t.removed
                    && t.source.origin == source.origin
                    && t.source.mid == source.mid
                    && t.source.kind == source.kind
            }) {
                existing.source.layers = source.layers.clone();
            } else {
                self.subscriptions.push(Subscription {
                    source: source.clone(),
                    state: TrackState::New,
                    removed: false,
                    announced: false,
                    max_height: 1080,
                    selected: None,
                    started: false,
                    last_switch: now,
                    clock: FrameClock::default(),
                });
            }
        }
        for message in messages {
            self.send(message);
        }
        let desired = self
            .subscriptions
            .iter()
            .filter(|t| {
                !t.removed && t.max_height > 0 && policy::media_kind(t.source.kind).is_video()
            })
            .map(|t| t.source.layers.last().map_or(2_000_000, |l| l.bitrate))
            .sum::<u64>()
            .saturating_add(128_000);
        self.rtc.bwe().set_desired_bitrate(Bitrate::bps(desired));
    }

    pub fn negotiate(&mut self, now: Instant) {
        if !self.initialized
            || !self.connected
            || self.pending.is_some()
            || now < self.negotiate_after
        {
            return;
        }
        let mut change = self.rtc.sdp_api();
        for track in &mut self.subscriptions {
            match track.state {
                TrackState::New if !track.removed => {
                    let mid = change.add_media(
                        policy::media_kind(track.source.kind),
                        Direction::SendOnly,
                        Some(track.source.origin.clone()),
                        None,
                        None,
                    );
                    track.state = TrackState::Adding(mid);
                }
                TrackState::Stop(mid) => {
                    change.stop_media(mid);
                    track.state = TrackState::Stopping(mid);
                }
                _ => {}
            }
        }
        if let Some((offer, pending)) = change.apply() {
            self.pending = Some(pending);
            self.pending_since = now;
            self.send(ServerMsg::Offer {
                sdp: opus_sdp(offer.to_sdp_string()),
            });
        }
    }

    pub fn request_keyframe(
        &mut self,
        mid: Mid,
        rid: Option<Rid>,
        now: Instant,
        kind: KeyframeRequestKind,
    ) {
        if self
            .keyframes
            .get(&(mid, rid))
            .is_some_and(|at| now.duration_since(*at) < Duration::from_millis(500))
        {
            return;
        }
        let Some(mut writer) = self.rtc.writer(mid) else {
            return;
        };
        if writer.request_keyframe(rid, kind).is_ok() {
            self.keyframes.insert((mid, rid), now);
        }
    }

    pub fn write_data(&mut self, data: &[u8]) {
        if let Some(mut channel) = self.cid.and_then(|cid| self.rtc.channel(cid)) {
            // Ephemeral messages can be dropped for a slow subscriber.
            if channel.buffered_amount() < 64 * 1024 {
                let _ = channel.write(false, data);
            }
        }
    }

    pub fn current_hook_state(&self) -> (bool, bool, bool, bool) {
        (
            self.muted,
            self.deafened,
            self.sources.values().any(|s| s.kind == TrackKind::Screen),
            self.sources.values().any(|s| s.kind == TrackKind::Camera),
        )
    }
}

pub(crate) fn parse_mid(value: &str) -> Option<Mid> {
    let mid: Mid = value.into();
    (!value.is_empty() && mid.to_string() == value).then_some(mid)
}

/// str0m 0.24 does not represent maxaveragebitrate. Advertise the receive
/// ceiling on the wire; the SFU relays Opus unchanged, including stereo/FEC.
fn opus_sdp(sdp: String) -> String {
    let mut opus = Vec::new();
    let mut output = String::with_capacity(sdp.len() + 128);
    for line in sdp.lines() {
        if line.starts_with("m=") {
            opus.clear();
        }
        if let Some(map) = line.strip_prefix("a=rtpmap:")
            && let Some((pt, codec)) = map.split_once(' ')
            && codec.eq_ignore_ascii_case("opus/48000/2")
        {
            opus.push(pt.to_owned());
        }
        output.push_str(line);
        if let Some(params) = line.strip_prefix("a=fmtp:")
            && let Some((pt, _)) = params.split_once(' ')
            && opus.iter().any(|opus_pt| opus_pt == pt)
        {
            for (key, value) in [
                ("stereo", "1"),
                ("sprop-stereo", "1"),
                ("maxaveragebitrate", "256000"),
            ] {
                let present = params
                    .split_once(' ')
                    .unwrap()
                    .1
                    .split(';')
                    .any(|p| p.trim().split_once('=').is_some_and(|(k, _)| k == key));
                if !present {
                    output.push_str(&format!(";{key}={value}"));
                }
            }
        }
        output.push_str("\r\n");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims() -> MediaClaims {
        MediaClaims {
            room: "room".into(),
            user: "alice".into(),
            name: "Alice".into(),
            color: "red".into(),
            can_speak: true,
            can_video: true,
            can_share: true,
            exp: f64::MAX,
        }
    }

    fn peer(claims: MediaClaims) -> (Peer, mpsc::Receiver<Signal>) {
        let (tx, rx) = mpsc::channel(128);
        (
            Peer::new(
                claims,
                tx,
                "127.0.0.1:7882".parse().unwrap(),
                &["127.0.0.1".parse().unwrap()],
                Instant::now(),
            )
            .unwrap(),
            rx,
        )
    }

    fn client() -> Rtc {
        Rtc::builder()
            .set_crypto_provider(Arc::new(str0m::crypto::from_feature_flags()))
            .build(Instant::now())
    }

    fn next_answer(rx: &mut mpsc::Receiver<Signal>) -> SdpAnswer {
        let Signal::Message(ServerMsg::Answer { sdp }) = rx.try_recv().unwrap() else {
            panic!("expected answer")
        };
        SdpAnswer::from_sdp_string(&sdp).unwrap()
    }

    #[test]
    fn sessions_expire_after_silence_and_closed_websockets() {
        let (mut peer, rx) = peer(claims());
        let now = Instant::now();
        peer.last_seen = now;
        assert!(!peer.expired(now + Duration::from_secs(29)));
        assert!(peer.expired(now + Duration::from_secs(31)));
        peer.last_seen = now + Duration::from_secs(30);
        assert!(!peer.expired(now + Duration::from_secs(31)));
        drop(rx);
        assert!(peer.expired(now + Duration::from_secs(31)));
    }

    #[test]
    fn capabilities_filter_declared_tracks_and_empty_snapshot_unpublishes() {
        let (mut peer, mut rx) = peer(MediaClaims {
            can_speak: false,
            can_video: false,
            can_share: true,
            ..claims()
        });
        let mut client = client();
        let mut change = client.sdp_api();
        let audio = change.add_media(
            str0m::media::MediaKind::Audio,
            Direction::SendOnly,
            None,
            None,
            None,
        );
        let camera = change.add_media(
            str0m::media::MediaKind::Video,
            Direction::SendOnly,
            None,
            None,
            None,
        );
        let screen = change.add_media(
            str0m::media::MediaKind::Video,
            Direction::SendOnly,
            None,
            None,
            None,
        );
        let (offer, pending) = change.apply().unwrap();
        peer.offer(&offer.to_sdp_string(), Instant::now()).unwrap();
        client
            .sdp_api()
            .accept_answer(pending, next_answer(&mut rx))
            .unwrap();
        peer.declare(vec![
            PublishedTrack {
                mid: audio.to_string(),
                kind: TrackKind::Audio,
            },
            PublishedTrack {
                mid: camera.to_string(),
                kind: TrackKind::Camera,
            },
            PublishedTrack {
                mid: screen.to_string(),
                kind: TrackKind::Screen,
            },
        ])
        .unwrap();
        assert!(peer.muted);
        assert_eq!(peer.sources.len(), 1);
        assert_eq!(peer.sources[&screen].kind, TrackKind::Screen);
        assert_eq!(peer.current_hook_state(), (true, false, true, false));
        assert!(
            peer.declare(vec![PublishedTrack {
                mid: audio.to_string(),
                kind: TrackKind::Screen
            }])
            .is_err()
        );
        assert_eq!(peer.sources.len(), 1, "invalid declarations are atomic");
        peer.declare(Vec::new()).unwrap();
        assert!(peer.sources.is_empty());
        assert_eq!(peer.current_hook_state(), (true, false, false, false));
    }

    #[test]
    fn client_offer_wins_collision_and_forwarding_offer_retries_once() {
        let (mut peer, mut rx) = peer(claims());
        let mut client = client();
        let now = Instant::now();
        let mut change = client.sdp_api();
        change.add_channel("pingo".into());
        let (offer, pending) = change.apply().unwrap();
        peer.offer(&offer.to_sdp_string(), now).unwrap();
        client
            .sdp_api()
            .accept_answer(pending, next_answer(&mut rx))
            .unwrap();
        peer.connected = true;
        let source = Source {
            origin: "bob".into(),
            mid: "0".into(),
            kind: TrackKind::Audio,
            layers: Vec::new(),
        };
        peer.reconcile(std::slice::from_ref(&source), now);
        peer.negotiate(now + Duration::from_secs(1));
        assert!(matches!(
            rx.try_recv(),
            Ok(Signal::Message(ServerMsg::Offer { .. }))
        ));
        assert!(peer.pending.is_some());
        // The client independently offers a microphone before receiving that offer.
        let mut change = client.sdp_api();
        change.add_media(
            str0m::media::MediaKind::Audio,
            Direction::SendOnly,
            None,
            None,
            None,
        );
        let (offer, pending) = change.apply().unwrap();
        peer.offer(&offer.to_sdp_string(), now + Duration::from_secs(1))
            .unwrap();
        client
            .sdp_api()
            .accept_answer(pending, next_answer(&mut rx))
            .unwrap();
        assert!(peer.pending.is_none());
        peer.reconcile(&[source], now);
        peer.negotiate(now + Duration::from_secs(2));
        let Signal::Message(ServerMsg::Offer { sdp }) = rx.try_recv().unwrap() else {
            panic!("expected retry")
        };
        peer.negotiate(now + Duration::from_secs(3));
        assert!(rx.try_recv().is_err(), "only one server offer in flight");
        let answer = client
            .sdp_api()
            .accept_offer(SdpOffer::from_sdp_string(&sdp).unwrap())
            .unwrap();
        peer.answer(&answer.to_sdp_string()).unwrap();
        assert_eq!(peer.subscriptions.len(), 1);
        assert!(peer.subscriptions[0].announced);
        assert!(matches!(
            rx.try_recv(),
            Ok(Signal::Message(ServerMsg::TrackPublished { .. }))
        ));
        peer.reconcile(&[], now);
        assert!(matches!(
            rx.try_recv(),
            Ok(Signal::Message(ServerMsg::TrackUnpublished { .. }))
        ));
        peer.negotiate(now + Duration::from_secs(4));
        let Signal::Message(ServerMsg::Offer { sdp }) = rx.try_recv().unwrap() else {
            panic!("expected stop offer")
        };
        let answer = client
            .sdp_api()
            .accept_offer(SdpOffer::from_sdp_string(&sdp).unwrap())
            .unwrap();
        peer.answer(&answer.to_sdp_string()).unwrap();
        assert!(peer.subscriptions.is_empty());
    }
}
