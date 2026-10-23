//! UDP host-candidate WebRTC client. `Session` is sans-I/O for deterministic tests;
//! `Client` supplies WebSocket/UDP transport and a 10 ms device-worker clock.
use crate::{
    Error, Result, StereoFrame,
    audio::{
        codec::{EncodedPacket, PacketDuration},
        device::DeviceIo,
    },
    pipeline::{AudioSettings, QualityReport, ReceivePipeline, SendPipeline},
};
use futures_util::{SinkExt, StreamExt};
use pingo_proto::{
    ClientMsg, DataMsg, ParticipantInfo, ServerMsg, TrackInfo, TrackKind, signal::PublishedTrack,
};
use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    time::{Duration, Instant},
};
use str0m::{
    Candidate, Event, Input, Output, Rtc,
    change::{SdpAnswer, SdpOffer, SdpPendingOffer},
    channel::ChannelId,
    format::{Codec, FormatParams},
    media::{Direction, Frequency, MediaKind, Mid},
    net::{Protocol, Receive},
    rtp::{Extension, ExtensionValues, RtpWrite},
};
use tokio::{
    net::{TcpStream, UdpSocket},
    sync::{broadcast, mpsc},
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};

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
    audio_mid: Option<Mid>,
    channel: Option<ChannelId>,
    joined: bool,
    pub connected: bool,
    tracks: HashMap<Mid, TrackInfo>,
    outgoing: VecDeque<ClientMsg>,
    events: VecDeque<RtcEvent>,
    pub receiver: ReceivePipeline,
    settings: AudioSettings,
    epoch: Instant,
    tx_sequence: Option<u64>,
    pub rtt_ms: f32,
    pub outgoing_loss_percent: f32,
}
impl Session {
    pub fn new(now: Instant, candidates: &[SocketAddr], settings: AudioSettings) -> Result<Self> {
        let mut config = Rtc::builder()
            .set_rtp_mode(true)
            .clear_codecs()
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
            audio_mid: None,
            channel: None,
            joined: false,
            connected: false,
            tracks: HashMap::new(),
            outgoing: VecDeque::new(),
            events: VecDeque::new(),
            receiver: ReceivePipeline::new(settings.clone()),
            settings,
            epoch: now,
            tx_sequence: None,
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
                self.events.push_back(RtcEvent::Joined {
                    participant_id,
                    participants,
                });
                let mut changes = self.rtc.sdp_api();
                let mid =
                    changes.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
                changes.add_channel("pingo".into());
                let (offer, pending) = changes
                    .apply()
                    .ok_or(Error::Invalid("empty initial offer"))?;
                self.audio_mid = Some(mid);
                self.pending = Some(pending);
                self.outgoing.push_back(ClientMsg::Offer {
                    sdp: offer.to_sdp_string(),
                });
                self.outgoing.push_back(ClientMsg::PublishTracks {
                    tracks: vec![PublishedTrack {
                        mid: mid.to_string(),
                        kind: TrackKind::Audio,
                    }],
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
                self.outgoing.push_back(ClientMsg::Answer {
                    sdp: answer.to_sdp_string(),
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
                self.events
                    .push_back(RtcEvent::ParticipantJoined(participant));
            }
            ServerMsg::ParticipantUpdated { participant } => {
                self.events
                    .push_back(RtcEvent::ParticipantUpdated(participant));
            }
            ServerMsg::ParticipantLeft { participant_id } => {
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
            Event::RtpPacket(packet) => {
                let mid = self
                    .rtc
                    .direct_api()
                    .stream_rx(&packet.header.ssrc)
                    .map(|s| s.mid());
                if let Some(track) = mid.and_then(|mid| self.tracks.get(&mid)) {
                    let samples = opus::packet::get_nb_samples(&packet.payload, 48_000);
                    let duration = match samples {
                        Ok(480) => PacketDuration::LowLatency10,
                        Ok(960) => PacketDuration::Default20,
                        _ => return Ok(()),
                    };
                    let audio_level = packet
                        .header
                        .ext_vals
                        .audio_level
                        .unwrap_or(-127)
                        .unsigned_abs()
                        .min(127);
                    let encoded = EncodedPacket {
                        sequence: packet.header.sequence_number,
                        timestamp: packet.header.timestamp,
                        duration,
                        payload: packet.payload.to_vec(),
                        audio_level,
                        speaking: packet.header.ext_vals.voice_activity.unwrap_or(false),
                    };
                    let arrival = packet
                        .timestamp
                        .saturating_duration_since(self.epoch)
                        .as_secs_f64()
                        * 1000.0;
                    if let Err(error) = self.receiver.push(&track_key(track), encoded, arrival) {
                        tracing::debug!(%error, "discarding invalid audio RTP");
                    }
                }
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
        if !self.connected {
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
        let sequence = self.tx_sequence.map_or(packet.sequence as u64, |high| {
            (high as i64 + packet.sequence.wrapping_sub(high as u16) as i16 as i64) as u64
        });
        self.tx_sequence = Some(sequence);
        let exts = ExtensionValues {
            audio_level: Some(-(packet.audio_level.min(127) as i8)),
            voice_activity: Some(packet.speaking),
            ..Default::default()
        };
        let mut direct = self.rtc.direct_api();
        let stream = direct
            .stream_tx_by_mid(mid, None)
            .ok_or(Error::Invalid("no audio sender"))?;
        stream.write_rtp(
            RtpWrite::new(pt, sequence.into(), packet.timestamp, now, packet.payload)
                .ext_vals(exts),
        );
        Ok(true)
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
    Capture(Box<StereoFrame>),
    Leave,
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
        mut self,
        mut devices: Option<DeviceIo>,
        mut commands: mpsc::Receiver<Command>,
    ) -> Result<()> {
        let mut tick = tokio::time::interval(Duration::from_millis(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut quality_tick = tokio::time::interval(Duration::from_secs(1));
        let mut buffer = [0u8; 65_536];
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
                    let output = self.session.receiver.render(now.duration_since(self.epoch).as_secs_f64()*1000.0)?;
                    self.sender.render_reference(&output)?;
                    if let Some(io) = devices.as_mut() {
                        if io.stats().fault && let Err(error) = io.recover() {
                            let _ = self.events.send(RtcEvent::DeviceError(error.to_string()));
                        }
                        io.playback(&output)?;
                        if let Some(capture) = io.capture()? && let Some(packet) = self.sender.process(&capture)? {
                            self.session.send_audio(packet, now)?;
                        }
                    }
                }
                _ = quality_tick.tick() => {
                    self.sender.feedback(self.session.outgoing_loss_percent)?;
                    let _ = self.events.send(RtcEvent::Quality(self.session.quality()));
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
                    Some(Command::Capture(frame)) => {
                        if let Some(packet) = self.sender.process(&frame)? { self.session.send_audio(packet, Instant::now())?; }
                    }
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
