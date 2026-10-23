//! UDP host-candidate WebRTC client. `Session` is sans-I/O for deterministic tests;
//! `Client` supplies WebSocket/UDP transport and a 10 ms device-worker clock.
use crate::{
    Error, Result, StereoFrame,
    audio::{
        codec::{EncodedPacket, PacketDuration},
        device::DeviceIo,
        jitter::JitterStats,
    },
    pipeline::{AudioSettings, QualityReport, ReceivePipeline, SendPipeline},
};
use futures_util::{SinkExt, StreamExt};
mod video;
use crate::video::{
    capture::CapturedFrame,
    encode::{EncodedVideoFrame, VideoCodec},
    simulcast::{VideoPublishConfig, VideoSource},
};
use pingo_proto::{
    ClientMsg, DataMsg, ParticipantInfo, ServerMsg, TrackInfo, TrackKind, signal::PublishedTrack,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::SocketAddr,
    time::{Duration, Instant},
};
use str0m::{
    Candidate, Event, Input, Output, Rtc,
    bwe::Bitrate,
    bwe::BweKind,
    change::{SdpAnswer, SdpOffer, SdpPendingOffer},
    channel::ChannelId,
    format::{Codec, FormatParams},
    media::{Direction, Frequency, MediaKind, Mid},
    media::{KeyframeRequestKind, MediaTime},
    net::{Protocol, Receive},
    rtp::Extension,
};
use tokio::{
    net::{TcpStream, UdpSocket},
    sync::{broadcast, mpsc},
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};
use video::LocalPublication;

fn rtc_error(error: impl std::fmt::Display) -> Error {
    Error::Rtc(error.to_string())
}

#[derive(Clone, Debug)]
pub enum RtcEvent {
    Joined {
        participant_id: String,
        participants: Vec<ParticipantInfo>,
    },
    ParticipantJoined(ParticipantInfo),
    ParticipantUpdated(ParticipantInfo),
    ParticipantLeft(String),
    TrackPublished(TrackInfo),
    TrackUnpublished {
        participant_id: String,
        mid: String,
    },
    /// Accepted SDP, useful for inspecting the negotiated codec and extensions.
    Negotiated {
        local_sdp: String,
        remote_sdp: String,
    },
    PlayoutStats(HashMap<String, JitterStats>),
    /// Complete depacketized encoded frame; H264 uses Annex B, VP8 is raw.
    /// Width/height are zero until the consumer parses the codec headers.
    VideoFrame {
        track: TrackInfo,
        frame: EncodedVideoFrame,
    },
    VideoKeyframeRequested {
        source: VideoSource,
        rid: Option<String>,
    },
    BandwidthEstimate {
        bits_per_second: u64,
    },
    Connected,
    Disconnected,
    Data(DataMsg),
    Quality(QualityReport),
    DeviceError(String),
    Error {
        code: String,
        message: String,
    },
}

pub struct Session {
    rtc: Rtc,
    pending: Option<SdpPendingOffer>,
    local_sdp: String,
    audio_mid: Option<Mid>,
    audio_published: bool,
    offered_audio: bool,
    negotiated_audio: bool,
    channel: Option<ChannelId>,
    joined: bool,
    pub connected: bool,
    tracks: HashMap<Mid, TrackInfo>,
    remote_muted: HashSet<String>,
    outgoing: VecDeque<ClientMsg>,
    events: VecDeque<RtcEvent>,
    pub receiver: ReceivePipeline,
    settings: AudioSettings,
    epoch: Instant,
    tx_timestamp: Option<u64>,
    publications: HashMap<TrackKind, LocalPublication>,
    pub rtt_ms: f32,
    pub outgoing_loss_percent: f32,
}
impl Session {
    pub fn new(now: Instant, candidates: &[SocketAddr], settings: AudioSettings) -> Result<Self> {
        let mut config = Rtc::builder()
            .clear_codecs()
            .enable_vp8(true)
            .enable_h264(true)
            .enable_bwe(Some(Bitrate::mbps(4)))
            .set_extension(1, Extension::AudioLevel)
            .set_stats_interval(Some(Duration::from_secs(1)));
        // Always negotiate stereo reception and permit Studio without renegotiation.
        // Voice's encoded payload still remains mono, 64 kbps and DTX-enabled.
        config.codec_config().add_config(
            111.into(),
            None,
            Codec::Opus,
            Frequency::FORTY_EIGHT_KHZ,
            Some(2),
            FormatParams {
                min_p_time: Some(10),
                stereo: Some(true),
                sprop_stereo: Some(true),
                use_inband_fec: Some(true),
                ..Default::default()
            },
        );
        let mut rtc = config.build(now);
        for address in candidates {
            rtc.add_local_candidate(Candidate::host(*address, "udp").map_err(rtc_error)?);
        }
        Ok(Self {
            rtc,
            pending: None,
            local_sdp: String::new(),
            audio_mid: None,
            audio_published: true,
            offered_audio: true,
            negotiated_audio: false,
            channel: None,
            joined: false,
            connected: false,
            tracks: HashMap::new(),
            remote_muted: HashSet::new(),
            outgoing: VecDeque::new(),
            events: VecDeque::new(),
            receiver: ReceivePipeline::new(settings.clone()),
            settings,
            epoch: now,
            tx_timestamp: None,
            publications: HashMap::new(),
            rtt_ms: 0.0,
            outgoing_loss_percent: 0.0,
        })
    }
    pub fn next_signal(&mut self) -> Option<ClientMsg> {
        self.outgoing.pop_front()
    }
    pub fn next_event(&mut self) -> Option<RtcEvent> {
        self.events.pop_front()
    }
    pub fn audio_mid(&self) -> Option<Mid> {
        self.audio_mid
    }

    pub fn signal(&mut self, message: ServerMsg) -> Result<()> {
        match message {
            ServerMsg::Joined {
                participant_id,
                participants,
            } => {
                if self.joined {
                    return Err(Error::Invalid("duplicate Joined"));
                }
                self.joined = true;
                for participant in &participants {
                    self.participant_state(participant)?;
                }
                self.events.push_back(RtcEvent::Joined {
                    participant_id,
                    participants,
                });
                let mut changes = self.rtc.sdp_api();
                let mid = changes.add_media(
                    MediaKind::Audio,
                    if self.audio_published {
                        Direction::SendOnly
                    } else {
                        Direction::Inactive
                    },
                    None,
                    None,
                    None,
                );
                changes.add_channel("pingo".into());
                let (offer, pending) = changes
                    .apply()
                    .ok_or(Error::Invalid("empty initial offer"))?;
                self.audio_mid = Some(mid);
                self.pending = Some(pending);
                self.offered_audio = self.audio_published;
                self.local_sdp = opus_sdp(offer.to_sdp_string());
                self.outgoing.push_back(ClientMsg::Offer {
                    sdp: self.local_sdp.clone(),
                });
                self.outgoing.push_back(ClientMsg::PublishTracks {
                    tracks: self.published_tracks(),
                });
                if self.settings.deafened || self.settings.gate == crate::audio::level::Gate::Muted
                {
                    self.outgoing.push_back(ClientMsg::SetState {
                        muted: self.settings.gate == crate::audio::level::Gate::Muted,
                        deafened: self.settings.deafened,
                    });
                }
            }
            ServerMsg::Answer { sdp } => {
                let answer = SdpAnswer::from_sdp_string(&sdp).map_err(rtc_error)?;
                let pending = self
                    .pending
                    .take()
                    .ok_or(Error::Invalid("unsolicited SDP answer"))?;
                self.rtc
                    .sdp_api()
                    .accept_answer(pending, answer)
                    .map_err(rtc_error)?;
                self.negotiated_audio = self.offered_audio;
                for publication in self.publications.values_mut() {
                    publication.negotiated = publication.offered.clone();
                }
                self.events.push_back(RtcEvent::Negotiated {
                    local_sdp: self.local_sdp.clone(),
                    remote_sdp: sdp,
                });
                self.offer_audio_change()?;
            }
            ServerMsg::Offer { sdp } => {
                // Contract: client offer wins glare; server retries after its answer.
                if self.pending.is_some() {
                    return Ok(());
                }
                if !self.joined {
                    return Err(Error::Invalid("offer before Joined"));
                }
                let offer = SdpOffer::from_sdp_string(&sdp).map_err(rtc_error)?;
                let answer = self.rtc.sdp_api().accept_offer(offer).map_err(rtc_error)?;
                self.local_sdp = opus_sdp(answer.to_sdp_string());
                self.events.push_back(RtcEvent::Negotiated {
                    local_sdp: self.local_sdp.clone(),
                    remote_sdp: sdp,
                });
                self.outgoing.push_back(ClientMsg::Answer {
                    sdp: self.local_sdp.clone(),
                });
            }
            ServerMsg::TrackPublished { track } => {
                if matches!(track.kind, TrackKind::Audio | TrackKind::ScreenAudio) {
                    let mid = Mid::from(track.mid.as_str());
                    // A track key keeps simultaneous mic/screen-audio SSRC timelines separate.
                    let key = track_key(&track);
                    self.receiver.add_participant(key)?;
                    if let Some(previous) = self.tracks.insert(mid, track.clone()) {
                        self.receiver.remove_participant(&track_key(&previous));
                        self.receiver.add_participant(track_key(&track))?;
                    }
                }
                self.tracks
                    .insert(Mid::from(track.mid.as_str()), track.clone());
                self.apply_receive_settings();
                self.events.push_back(RtcEvent::TrackPublished(track));
            }
            ServerMsg::TrackUnpublished {
                participant_id,
                mid,
            } => {
                if let Some(track) = self.tracks.remove(&Mid::from(mid.as_str())) {
                    self.receiver.remove_participant(&track_key(&track));
                }
                self.events.push_back(RtcEvent::TrackUnpublished {
                    participant_id,
                    mid,
                });
            }
            ServerMsg::ParticipantJoined { participant } => {
                self.participant_state(&participant)?;
                self.events
                    .push_back(RtcEvent::ParticipantJoined(participant));
            }
            ServerMsg::ParticipantUpdated { participant } => {
                self.participant_state(&participant)?;
                self.events
                    .push_back(RtcEvent::ParticipantUpdated(participant));
            }
            ServerMsg::ParticipantLeft { participant_id } => {
                self.remote_muted.remove(&participant_id);
                self.tracks.retain(|_, track| {
                    if track.participant_id == participant_id {
                        self.receiver.remove_participant(&track_key(track));
                        false
                    } else {
                        true
                    }
                });
                self.events
                    .push_back(RtcEvent::ParticipantLeft(participant_id));
            }
            ServerMsg::Error { code, message } => {
                self.events.push_back(RtcEvent::Error { code, message });
            }
            ServerMsg::Pong => {}
        }
        Ok(())
    }

    fn event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Connected => {
                self.connected = true;
                self.events.push_back(RtcEvent::Connected);
            }
            Event::Closed => {
                self.connected = false;
                self.events.push_back(RtcEvent::Disconnected);
            }
            Event::IceConnectionStateChange(str0m::IceConnectionState::Disconnected) => {
                self.connected = false;
                self.events.push_back(RtcEvent::Disconnected);
            }
            Event::ChannelOpen(id, name) if name == "pingo" => self.channel = Some(id),
            Event::ChannelClose(id) if self.channel == Some(id) => self.channel = None,
            Event::ChannelData(data) if self.channel == Some(data.id) => {
                if let Ok(message) = serde_json::from_slice::<DataMsg>(&data.data) {
                    self.events.push_back(RtcEvent::Data(message));
                }
            }
            Event::PeerStats(stats) => {
                if let Some(rtt) = stats.rtt.or_else(|| {
                    stats
                        .selected_candidate_pair
                        .and_then(|p| p.current_round_trip_time)
                }) {
                    self.rtt_ms = rtt.as_secs_f32() * 1000.0;
                }
                if let Some(loss) = stats.egress_loss_fraction {
                    self.outgoing_loss_percent = loss * 100.0;
                }
            }
            Event::MediaData(data) => {
                if let Some(track) = self.tracks.get(&data.mid).cloned() {
                    if matches!(track.kind, TrackKind::Camera | TrackKind::Screen) {
                        let codec = match data.params.spec().codec {
                            Codec::H264 => VideoCodec::H264,
                            Codec::Vp8 => VideoCodec::Vp8,
                            _ => return Ok(()),
                        };
                        let keyframe = data.is_keyframe();
                        if !data.contiguous {
                            self.request_video_keyframe(&track.mid)?;
                        }
                        self.events.push_back(RtcEvent::VideoFrame {
                            track,
                            frame: EncodedVideoFrame {
                                codec,
                                data: data.data,
                                timestamp: Duration::from_secs_f64(data.time.as_seconds()),
                                keyframe,
                                width: 0,
                                height: 0,
                                rid: data.rid.map(|rid| rid.to_string()),
                            },
                        });
                    } else {
                        let duration = match opus::packet::get_nb_samples(&data.data, 48_000) {
                            Ok(480) => PacketDuration::LowLatency10,
                            Ok(960) => PacketDuration::Default20,
                            _ => return Ok(()),
                        };
                        let encoded = EncodedPacket {
                            sequence: **data.seq_range.start() as u16,
                            timestamp: data.time.rebase(Frequency::FORTY_EIGHT_KHZ).numer() as u32,
                            duration,
                            payload: data.data.to_vec(),
                            audio_level: data
                                .ext_vals
                                .audio_level
                                .unwrap_or(-127)
                                .unsigned_abs()
                                .min(127),
                            speaking: data.ext_vals.voice_activity.unwrap_or(false),
                        };
                        let arrival = data
                            .network_time
                            .saturating_duration_since(self.epoch)
                            .as_secs_f64()
                            * 1000.0;
                        if let Err(error) = self.receiver.push(&track_key(&track), encoded, arrival)
                        {
                            tracing::debug!(%error,"discarding invalid audio frame");
                        }
                    }
                }
            }
            Event::KeyframeRequest(request) => {
                if let Some((kind, _)) = self
                    .publications
                    .iter()
                    .find(|(_, p)| p.mid == Some(request.mid))
                    && let Some(source) = video::source_kind(*kind)
                {
                    self.events.push_back(RtcEvent::VideoKeyframeRequested {
                        source,
                        rid: request.rid.map(|rid| rid.to_string()),
                    });
                }
            }
            Event::EgressBitrateEstimate(
                BweKind::Twcc { estimate, .. } | BweKind::Remb { estimate, .. },
            ) => {
                self.events.push_back(RtcEvent::BandwidthEstimate {
                    bits_per_second: estimate.as_u64(),
                });
            }
            _ => {}
        }
        Ok(())
    }
    /// Consumes library events internally. Drain to Timeout after every mutation.
    pub fn poll_output(&mut self) -> Result<Output> {
        loop {
            match self.rtc.poll_output().map_err(rtc_error)? {
                Output::Event(event) => self.event(event)?,
                output => return Ok(output),
            }
        }
    }
    pub fn timeout(&mut self, now: Instant) -> Result<()> {
        self.rtc
            .handle_input(Input::Timeout(now))
            .map_err(rtc_error)
    }
    pub fn receive(
        &mut self,
        now: Instant,
        source: SocketAddr,
        destination: SocketAddr,
        data: &[u8],
    ) -> Result<()> {
        let receive = Receive::new(Protocol::Udp, source, destination, data).map_err(rtc_error)?;
        let input = Input::Receive(now, receive);
        if self.rtc.accepts(&input) {
            self.rtc.handle_input(input).map_err(rtc_error)?;
        }
        Ok(())
    }
    pub fn send_audio(&mut self, packet: EncodedPacket, now: Instant) -> Result<bool> {
        if !self.connected || !self.audio_published || !self.negotiated_audio {
            return Ok(false);
        }
        let mid = self
            .audio_mid
            .ok_or(Error::Invalid("no local audio track"))?;
        let media = self
            .rtc
            .media(mid)
            .ok_or(Error::Invalid("audio not negotiated"))?;
        let pt = self
            .rtc
            .codec_config()
            .params()
            .iter()
            .find(|p| p.spec().codec == Codec::Opus && media.remote_pts().contains(&p.pt()))
            .map(|p| p.pt())
            .ok_or(Error::Invalid("remote did not accept Opus"))?;
        let timestamp = self.tx_timestamp.map_or(packet.timestamp as u64, |high| {
            (high as i64 + packet.timestamp.wrapping_sub(high as u32) as i32 as i64).max(0) as u64
        });
        self.tx_timestamp = Some(timestamp);
        self.rtc
            .writer(mid)
            .ok_or(Error::Invalid("no audio sender"))?
            .audio_level(-(packet.audio_level.min(127) as i8), packet.speaking)
            .write(
                pt,
                now,
                MediaTime::new(timestamp, Frequency::FORTY_EIGHT_KHZ),
                packet.payload,
            )
            .map_err(rtc_error)?;
        Ok(true)
    }
    /// PublishTracks is a full snapshot. Inactive (rather than stopped) media
    /// can be republished on the same source mid, preserving the RTP clock.
    pub fn set_audio_published(&mut self, published: bool) -> Result<()> {
        self.audio_published = published;
        self.offer_audio_change()
    }

    pub fn send_data(&mut self, message: &DataMsg) -> Result<bool> {
        let Some(id) = self.channel else {
            return Ok(false);
        };
        let bytes = serde_json::to_vec(message)?;
        if bytes.len() > 16_384 {
            return Err(Error::Invalid("data message too large"));
        }
        self.rtc
            .channel(id)
            .ok_or(Error::Invalid("data channel closed"))?
            .write(false, &bytes)
            .map_err(rtc_error)
    }
    pub fn quality(&self) -> QualityReport {
        let stats = self.receiver.stats();
        let jitter = stats.values().map(|s| s.jitter_ms).fold(0.0, f32::max);
        let loss = stats
            .values()
            .map(|s| s.loss_percent)
            .fold(self.outgoing_loss_percent, f32::max);
        let delay = stats
            .values()
            .map(|s| s.current_delay_ms)
            .fold(0.0, f32::max);
        QualityReport::new(loss, jitter, self.rtt_ms, delay)
    }
    pub fn set_settings(&mut self, settings: AudioSettings) {
        self.outgoing.push_back(ClientMsg::SetState {
            muted: settings.gate == crate::audio::level::Gate::Muted,
            deafened: settings.deafened,
        });
        self.settings = settings;
        self.apply_receive_settings();
    }
    fn participant_state(&mut self, participant: &ParticipantInfo) -> Result<()> {
        let changed = if participant.muted {
            self.remote_muted.insert(participant.participant_id.clone())
        } else {
            self.remote_muted.remove(&participant.participant_id)
        };
        if changed {
            // A deliberate mute is not packet loss. Drop queued speech/PLC and
            // re-anchor the remote timeline when it resumes.
            for track in self.tracks.values().filter(|t| {
                t.participant_id == participant.participant_id
                    && matches!(t.kind, TrackKind::Audio | TrackKind::ScreenAudio)
            }) {
                let key = track_key(track);
                self.receiver.remove_participant(&key);
                self.receiver.add_participant(key)?;
            }
        }
        self.apply_receive_settings();
        Ok(())
    }

    fn apply_receive_settings(&mut self) {
        let mut settings = self.settings.clone();
        // UI volume/panning is participant-based, while playout buffers are track-based.
        for track in self.tracks.values() {
            if let Some(volume) = settings
                .per_user_volumes
                .get(&track.participant_id)
                .copied()
            {
                settings.per_user_volumes.insert(track_key(track), volume);
            }
            if self.remote_muted.contains(&track.participant_id) {
                settings.per_user_volumes.insert(track_key(track), 0.0);
            }
            if let Some(pan) = settings.per_user_pan.get(&track.participant_id).copied() {
                settings.per_user_pan.insert(track_key(track), pan);
            }
        }
        self.receiver.set_settings(settings);
    }
}
fn track_key(track: &TrackInfo) -> String {
    format!("{}:{}", track.participant_id, track.mid)
}

#[derive(Clone, Debug)]
pub struct ClientConfig {
    pub url: String,
    pub token: String,
    pub bind: SocketAddr,
    pub settings: AudioSettings,
}
impl ClientConfig {
    pub fn new(url: String, token: String) -> Self {
        Self {
            url,
            token,
            bind: "0.0.0.0:0".parse().unwrap(),
            settings: AudioSettings::default(),
        }
    }
}
#[derive(Debug)]
pub enum Command {
    Settings(Box<AudioSettings>),
    Data(DataMsg),
    PublishAudio(bool),
    Capture(Box<StereoFrame>),
    /// Configuring None unpublishes; each start sends a full PublishTracks snapshot.
    PublishVideo {
        source: VideoSource,
        config: Option<VideoPublishConfig>,
    },
    /// Raw capture enters a bounded encoding worker, separate from audio/UDP.
    VideoFrame {
        source: VideoSource,
        frame: Box<CapturedFrame>,
    },
    /// External encoders may bypass the built-in worker.
    EncodedVideo {
        source: VideoSource,
        frame: EncodedVideoFrame,
    },
    PublishScreenAudio(bool),
    CaptureScreenAudio(Box<StereoFrame>),
    SetVideoQuality {
        mid: String,
        max_height: u32,
    },
    RequestVideoKeyframe {
        mid: String,
    },
    Leave,
}

/// The exact mixed 10 ms output and its worker-clock timestamp. External sinks
/// should consume this bounded stream continuously, like an audio callback.
pub struct PlaybackFrame {
    pub at: Instant,
    pub frame: Box<StereoFrame>,
}

pub struct Client {
    session: Session,
    socket: UdpSocket,
    websocket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    sender: SendPipeline,
    settings: AudioSettings,
    epoch: Instant,
    local_address: SocketAddr,
    routes: HashMap<SocketAddr, SocketAddr>,
    pub events: broadcast::Sender<RtcEvent>,
}
impl Client {
    pub async fn connect(config: ClientConfig) -> Result<Self> {
        let socket = UdpSocket::bind(config.bind).await?;
        let local_address = socket.local_addr()?;
        let candidates: Vec<_> = if local_address.ip().is_unspecified() {
            if_addrs::get_if_addrs()?
                .iter()
                .map(|interface| interface.ip())
                .filter(|ip| ip.is_ipv4() == local_address.is_ipv4() && !ip.is_unspecified())
                .map(|ip| SocketAddr::new(ip, local_address.port()))
                .collect()
        } else {
            vec![local_address]
        };
        let epoch = Instant::now();
        let session = Session::new(epoch, &candidates, config.settings.clone())?;
        let sender = SendPipeline::new(config.settings.clone())?;
        let (mut websocket, _) = tokio_tungstenite::connect_async(&config.url).await?;
        websocket
            .send(Message::Text(
                serde_json::to_string(&ClientMsg::Join {
                    token: config.token,
                })?
                .into(),
            ))
            .await?;
        let (events, _) = broadcast::channel(256);
        Ok(Self {
            session,
            socket,
            websocket,
            sender,
            settings: config.settings,
            epoch,
            local_address,
            routes: HashMap::new(),
            events,
        })
    }

    /// Run on a dedicated worker/runtime thread when embedded in Tauri. CPAL's
    /// callbacks remain independent of socket I/O and all processing allocations.
    pub async fn run(
        self,
        devices: Option<DeviceIo>,
        commands: mpsc::Receiver<Command>,
    ) -> Result<()> {
        self.run_with_playback(devices, commands, None).await
    }

    /// Hardware-free external I/O uses Capture commands and a bounded playback
    /// sink. A slow sink drops frames instead of stalling ICE, audio or signaling.
    pub async fn run_with_playback(
        mut self,
        mut devices: Option<DeviceIo>,
        mut commands: mpsc::Receiver<Command>,
        playback: Option<mpsc::Sender<PlaybackFrame>>,
    ) -> Result<()> {
        let mut tick = tokio::time::interval(Duration::from_millis(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut quality_tick = tokio::time::interval(Duration::from_secs(1));
        let mut buffer = [0u8; 65_536];
        let mut video_workers = HashMap::<VideoSource, crate::video::worker::EncoderWorker>::new();
        let mut screen_audio_sender = None::<SendPipeline>;
        loop {
            let deadline = loop {
                match self.session.poll_output()? {
                    Output::Transmit(tx) => {
                        self.socket.send_to(&tx.contents, tx.destination).await?;
                    }
                    Output::Timeout(deadline) => break deadline,
                    Output::Event(_) => unreachable!("Session consumes events"),
                }
            };
            while let Some(message) = self.session.next_signal() {
                self.websocket
                    .send(Message::Text(serde_json::to_string(&message)?.into()))
                    .await?;
            }
            while let Some(event) = self.session.next_event() {
                match &event {
                    RtcEvent::VideoKeyframeRequested { source, rid } => {
                        if let Some(worker) = video_workers.get(source) {
                            worker.keyframe(rid.as_deref());
                        }
                    }
                    RtcEvent::BandwidthEstimate { bits_per_second } => {
                        let per_video = bits_per_second / video_workers.len().max(1) as u64;
                        for worker in video_workers.values() {
                            worker.bandwidth(per_video);
                        }
                    }
                    _ => {}
                }
                let _ = self.events.send(event);
            }
            tokio::select! {
                message = self.websocket.next() => {
                    match message {
                        Some(Ok(Message::Text(text))) => self.session.signal(serde_json::from_str(&text)?)?,
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Err(error)) => return Err(error.into()),
                        _ => {}
                    }
                }
                received = self.socket.recv_from(&mut buffer) => {
                    let (len, source) = received?;
                    // For a wildcard socket, ICE matches the source against the
                    // advertised local addresses; use the selected local route.
                    let destination = if self.local_address.ip().is_unspecified() {
                        if let Some(address) = self.routes.get(&source) { *address } else {
                            let address = route_address(source, self.local_address.port())?;
                            if self.routes.len() < 64 { self.routes.insert(source, address); }
                            address
                        }
                    } else { self.local_address };
                    if let Err(error) = self.session.receive(Instant::now(), source, destination, &buffer[..len]) {
                        tracing::debug!(%error, "discarding invalid UDP datagram");
                    }
                }
                _ = tokio::time::sleep_until(deadline.into()) => self.session.timeout(Instant::now())?,
                _ = tick.tick() => {
                    let now = Instant::now();
                    for (source,worker) in &mut video_workers {
                        while let Some(packet)=worker.next() {
                            match packet {
                                Ok(frame)=>{ self.session.send_video(*source,frame,now)?; }
                                Err(error)=>{ let _=self.events.send(RtcEvent::DeviceError(error.to_string())); }
                            }
                        }
                    }
                    let output = self.session.receiver.render(now.duration_since(self.epoch).as_secs_f64()*1000.0)?;
                    let mut played = false;
                    if let Some(sink) = &playback {
                        played |= sink.try_send(PlaybackFrame { at: now, frame: Box::new(output) }).is_ok();
                    }
                    if let Some(io) = devices.as_mut() {
                        if io.stats().fault && let Err(error) = io.recover() {
                            let _ = self.events.send(RtcEvent::DeviceError(error.to_string()));
                        }
                        self.sender.set_stream_delay_ms(Some(io.stream_delay_ms()));
                        played |= io.playback(&output)?;
                        self.sender.render_reference(if played { &output } else { &[[0.0; 2]; crate::FRAME_SAMPLES] })?;
                        if let Some(capture) = io.capture()? && let Some(packet) = self.sender.process(&capture)? {
                            self.session.send_audio(packet, now)?;
                        }
                    } else {
                        self.sender.render_reference(if played { &output } else { &[[0.0; 2]; crate::FRAME_SAMPLES] })?;
                    }
                }
                _ = quality_tick.tick() => {
                    self.sender.feedback(self.session.outgoing_loss_percent)?;
                    let _ = self.events.send(RtcEvent::Quality(self.session.quality()));
                    let _ = self.events.send(RtcEvent::PlayoutStats(self.session.receiver.stats()));
                }
                command = commands.recv() => match command {
                    Some(Command::Settings(settings)) => {
                        if let Some(io) = devices.as_mut() && (settings.input_device != self.settings.input_device || settings.output_device != self.settings.output_device)
                            && let Err(error) = io.select_devices(settings.input_device.clone(), settings.output_device.clone()) {
                                let _ = self.events.send(RtcEvent::DeviceError(error.to_string()));
                                continue;
                        }
                        self.sender.set_settings((*settings).clone())?;
                        self.session.set_settings((*settings).clone());
                        self.settings = *settings;
                    }
                    Some(Command::Data(data)) => { self.session.send_data(&data)?; }
                    Some(Command::PublishAudio(published)) => self.session.set_audio_published(published)?,
                    Some(Command::Capture(frame)) => {
                        if let Some(packet) = self.sender.process(&frame)? { self.session.send_audio(packet, Instant::now())?; }
                    }
                    Some(Command::PublishVideo { source,config }) => {
                        if let Some(config)=config {
                            if config.source!=source { return Err(Error::Invalid("video source/config mismatch")); }
                            let worker=crate::video::worker::EncoderWorker::new(config.clone()).map_err(rtc_error)?;
                            self.session.set_video_published(source,Some(config))?;
                            video_workers.insert(source,worker);
                        } else {
                            self.session.set_video_published(source,None)?;
                            video_workers.remove(&source);
                        }
                    }
                    Some(Command::VideoFrame { source,frame }) => { if let Some(worker)=video_workers.get(&source) { worker.frame(frame); } }
                    Some(Command::EncodedVideo { source,frame }) => { self.session.send_video(source,frame,Instant::now())?; }
                    Some(Command::PublishScreenAudio(published)) => {
                        self.session.set_screen_audio_published(published)?;
                        if published && screen_audio_sender.is_none() {
                            screen_audio_sender=Some(SendPipeline::new(AudioSettings {
                                profile:crate::audio::codec::Profile::Studio,gate:crate::audio::level::Gate::Open,
                                noise_level:crate::audio::denoise::NoiseLevel::Off,agc:false,normalize:false,..Default::default()
                            })?);
                        }
                    }
                    Some(Command::CaptureScreenAudio(frame)) => {
                        if let Some(sender)=&mut screen_audio_sender && let Some(packet)=sender.process(&frame)? {
                            self.session.send_screen_audio(packet,Instant::now())?;
                        }
                    }
                    Some(Command::SetVideoQuality { mid,max_height }) => self.session.set_video_quality(mid,max_height),
                    Some(Command::RequestVideoKeyframe { mid }) => self.session.request_video_keyframe(&mid)?,
                    Some(Command::Leave) | None => break,
                },
            }
        }
        let _ = self.websocket.close(None).await;
        let _ = self.events.send(RtcEvent::Disconnected);
        Ok(())
    }
}

fn route_address(remote: SocketAddr, port: u16) -> Result<SocketAddr> {
    // UDP connect selects a local route without sending packets or resolving DNS.
    let probe = std::net::UdpSocket::bind(if remote.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    })?;
    probe.connect(remote)?;
    Ok(SocketAddr::new(probe.local_addr()?.ip(), port))
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
