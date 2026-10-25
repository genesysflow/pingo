//! Call and loopback workers own CPAL and RTC on a dedicated current-thread runtime.
use pingo_media::{
    audio::{
        codec::Profile,
        denoise::NoiseLevel,
        device::{DeviceIo, list_devices},
        level::Gate,
    },
    pipeline::{AudioSettings, ReceivePipeline, SendPipeline},
    rtc::{Client, ClientConfig, Command, RtcEvent},
};
use pingo_proto::DataMsg;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};

#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("No active native call")]
    Idle,
    #[error("Native call worker is busy or closed")]
    Busy,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Settings {
    pub microphone: String,
    pub speaker: String,
    pub suppression: String,
    pub studio: bool,
    pub echo_cancel: bool,
    pub muted: bool,
    pub deafened: bool,
    pub push_to_talk: bool,
    pub ptt_pressed: bool,
    pub volumes: HashMap<String, Mix>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Mix {
    pub volume: f32,
    pub pan: f32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            microphone: "default".into(),
            speaker: "default".into(),
            suppression: "Gentle".into(),
            studio: false,
            echo_cancel: true,
            muted: false,
            deafened: false,
            push_to_talk: false,
            ptt_pressed: false,
            volumes: HashMap::new(),
        }
    }
}
impl Settings {
    pub fn audio(&self) -> Result<AudioSettings, CallError> {
        let noise_level = match self.suppression.as_str() {
            "Off" => NoiseLevel::Off,
            "Gentle" => NoiseLevel::Gentle,
            "Strong" => NoiseLevel::Strong,
            _ => return Err(CallError::Invalid("Unknown noise suppression level")),
        };
        if self.microphone.len() > 1024 || self.speaker.len() > 1024 || self.volumes.len() > 1024 {
            return Err(CallError::Invalid("Audio settings exceed limits"));
        }
        for (user, mix) in &self.volumes {
            if user.len() > 256
                || !mix.volume.is_finite()
                || !(0.0..=1.5).contains(&mix.volume)
                || !mix.pan.is_finite()
                || !(-1.0..=1.0).contains(&mix.pan)
            {
                return Err(CallError::Invalid(
                    "Volume must be 0–1.5 and pan must be -1–1",
                ));
            }
        }
        Ok(AudioSettings {
            profile: if self.studio {
                Profile::Studio
            } else {
                Profile::Voice
            },
            noise_level,
            echo_cancel: self.echo_cancel && !self.studio,
            agc: !self.studio,
            gate: if self.muted || self.deafened {
                Gate::Muted
            } else if self.push_to_talk {
                Gate::PushToTalk {
                    pressed: self.ptt_pressed,
                }
            } else {
                Gate::VoiceActivation
            },
            deafened: self.deafened,
            input_device: (self.microphone != "default").then(|| self.microphone.clone()),
            output_device: (self.speaker != "default").then(|| self.speaker.clone()),
            per_user_volumes: self
                .volumes
                .iter()
                .map(|(id, m)| (id.clone(), m.volume))
                .collect(),
            per_user_pan: self
                .volumes
                .iter()
                .map(|(id, m)| (id.clone(), m.pan))
                .collect(),
            ..Default::default()
        })
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallEvent {
    pub session_id: String,
    pub event: Value,
}
pub type EventSink = Arc<dyn Fn(CallEvent) + Send + Sync>;
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub device_id: String,
    pub label: String,
    pub kind: &'static str,
    pub is_default: bool,
}
pub fn devices() -> Result<Vec<Device>, String> {
    list_devices()
        .map(|devices| {
            devices
                .into_iter()
                .map(|d| Device {
                    device_id: d.name.clone(),
                    label: d.name,
                    kind: if d.input { "audioinput" } else { "audiooutput" },
                    is_default: d.is_default,
                })
                .collect()
        })
        .map_err(|e| e.to_string())
}
struct Active {
    id: String,
    commands: mpsc::Sender<Command>,
    stop: watch::Sender<bool>,
    loopback: bool,
}
#[derive(Default)]
struct State {
    active: Option<Active>,
    settings: Settings,
}
pub struct JoinRequest<'a> {
    pub id: String,
    pub origin: &'a str,
    pub url: &'a str,
    pub token: String,
    pub settings: Settings,
    pub video_sink: Option<crate::video::PacketSink>,
}
#[derive(Default)]
pub struct CallManager {
    state: Arc<Mutex<State>>,
}
impl CallManager {
    pub fn settings(&self) -> Settings {
        self.state.lock().unwrap().settings.clone()
    }
    pub fn configure(&self, id: &str, mut settings: Settings) -> Result<(), CallError> {
        let mut state = self.state.lock().unwrap();
        let active = state
            .active
            .as_ref()
            .filter(|a| a.id == id)
            .ok_or(CallError::Idle)?;
        settings.ptt_pressed = state.settings.ptt_pressed && settings.push_to_talk;
        let audio = settings.audio()?;
        active
            .commands
            .try_send(Command::Settings(Box::new(audio)))
            .map_err(|_| CallError::Busy)?;
        state.settings = settings;
        Ok(())
    }
    pub fn ptt(&self, pressed: bool) -> Result<(), CallError> {
        let mut state = self.state.lock().unwrap();
        let mut next = state.settings.clone();
        next.ptt_pressed = pressed;
        if let Some(active) = &state.active {
            active
                .commands
                .try_send(Command::Settings(Box::new(next.audio()?)))
                .map_err(|_| CallError::Busy)?;
        }
        state.settings = next;
        Ok(())
    }
    pub fn toggle_mute(&self) -> Result<Settings, CallError> {
        let mut state = self.state.lock().unwrap();
        let mut next = state.settings.clone();
        next.muted = !next.muted;
        if let Some(active) = &state.active {
            active
                .commands
                .try_send(Command::Settings(Box::new(next.audio()?)))
                .map_err(|_| CallError::Busy)?;
        }
        state.settings = next.clone();
        Ok(next)
    }
    pub fn send(&self, id: &str, mut data: DataMsg) -> Result<(), CallError> {
        if matches!(data, DataMsg::Speaking { .. }) {
            return Err(CallError::Invalid("Speaking levels come from the SFU"));
        }
        if serde_json::to_vec(&data).map_or(true, |b| b.len() > 16_384) {
            return Err(CallError::Invalid("Data message exceeds 16 KiB"));
        }
        // The SFU authenticates/stamps this field; never trust the UI's identity.
        match &mut data {
            DataMsg::Reaction { participant_id, .. }
            | DataMsg::RaiseHand { participant_id, .. }
            | DataMsg::Pointer { participant_id, .. }
            | DataMsg::Stroke { participant_id, .. } => participant_id.clear(),
            _ => {}
        }
        let state = self.state.lock().unwrap();
        state
            .active
            .as_ref()
            .filter(|a| a.id == id && !a.loopback)
            .ok_or(CallError::Idle)?
            .commands
            .try_send(Command::Data(data))
            .map_err(|_| CallError::Busy)
    }
    pub fn is_active(&self, id: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .active
            .as_ref()
            .is_some_and(|a| a.id == id)
    }
    pub fn video_quality(&self, id: &str, mid: String, height: u32) -> Result<(), CallError> {
        if mid.is_empty() || mid.len() > 64 || height > 4320 {
            return Err(CallError::Invalid("Invalid video quality preference"));
        }
        let state = self.state.lock().unwrap();
        state
            .active
            .as_ref()
            .filter(|a| a.id == id)
            .ok_or(CallError::Idle)?
            .commands
            .try_send(Command::SetVideoQuality {
                mid,
                max_height: height,
            })
            .map_err(|_| CallError::Busy)
    }
    pub fn leave(&self, id: Option<&str>) {
        let mut state = self.state.lock().unwrap();
        if state
            .active
            .as_ref()
            .is_some_and(|a| id.is_none_or(|id| id == a.id))
        {
            let active = state.active.take().unwrap();
            let _ = active.stop.send(true);
            let _ = active.commands.try_send(Command::Leave);
        }
    }
    /// Admission is supplied by the UI, but may only be sent back to its configured server.
    pub fn join(
        &self,
        request: JoinRequest<'_>,
        sink: EventSink,
        hardware: bool,
    ) -> Result<(), CallError> {
        let JoinRequest {
            id,
            origin,
            url,
            token,
            settings,
            video_sink,
        } = request;
        let url = rtc_url(origin, url)?;
        if token.is_empty() || token.len() > 16_384 || id.is_empty() || id.len() > 128 {
            return Err(CallError::Invalid("Invalid call ID or admission token"));
        }
        let audio = settings.audio()?;
        self.start(
            id,
            settings,
            false,
            sink,
            move |commands, stop, sink, id| {
                Box::pin(async move {
                    let mut config = ClientConfig::new(url, token);
                    config.settings = audio.clone();
                    let client =
                        tokio::time::timeout(Duration::from_secs(15), Client::connect(config))
                            .await
                            .map_err(|_| "Native call connection timed out".to_string())?
                            .map_err(|e| e.to_string())?;
                    let events = client.events.subscribe();
                    let io = if hardware {
                        Some(
                            DeviceIo::open(audio.input_device, audio.output_device)
                                .map_err(|e| e.to_string())?,
                        )
                    } else {
                        None
                    };
                    let forward = forward_events(events, sink.clone(), id.clone(), video_sink);
                    tokio::pin!(forward);
                    let (playback, mut frames) = mpsc::channel::<pingo_media::rtc::PlaybackFrame>(8);
                    let observe = async {
                        let mut bytes = 0u64;
                        let mut last = Instant::now();
                        while let Some(frame) = frames.recv().await {
                            if frame.frame.iter().any(|s| s[0].abs() > 0.00001 || s[1].abs() > 0.00001) { bytes += (pingo_media::FRAME_SAMPLES*2*4) as u64; }
                            if last.elapsed() >= Duration::from_secs(1) {
                                sink(CallEvent { session_id: id.clone(), event: json!({"type":"playback", "receivedPcmBytes":bytes}) });
                                last = Instant::now();
                            }
                        }
                    };
                    tokio::pin!(observe);
                    let mut stop = stop;
                    tokio::select! {
                        result = client.run_with_playback(io, commands, Some(playback)) => result.map_err(|e| e.to_string()),
                        _ = &mut observe => Err("Playback observer closed".into()),
                        _ = &mut forward => Err("Native event stream closed".into()),
                        _ = stop.changed() => Ok(()),
                    }
                })
            },
        )
    }
    pub fn loopback(
        &self,
        id: String,
        mut settings: Settings,
        sink: EventSink,
    ) -> Result<(), CallError> {
        if self.state.lock().unwrap().active.is_some() {
            return Err(CallError::Invalid(
                "Leave the call before starting sound check",
            ));
        }
        settings.muted = false;
        settings.deafened = false;
        settings.push_to_talk = false;
        let mut audio = settings.audio()?;
        audio.gate = Gate::Open;
        self.start(id, settings, true, sink, move |_, stop, sink, id| Box::pin(async move {
            let mut io = DeviceIo::open(audio.input_device.clone(), audio.output_device.clone()).map_err(|e| e.to_string())?;
            let mut sender = SendPipeline::new(audio.clone()).map_err(|e| e.to_string())?;
            let mut receiver = ReceivePipeline::new(audio);
            receiver.add_participant("loopback".into()).map_err(|e| e.to_string())?;
            let start = Instant::now();
            let mut tick = tokio::time::interval(Duration::from_millis(10));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut stop = stop;
            let mut count = 0;
            loop {
                tokio::select! { _ = tick.tick() => {}, _ = stop.changed() => break }
                let now = start.elapsed().as_secs_f64() * 1000.0;
                if now >= 5000.0 { break; }
                if io.stats().fault { io.recover().map_err(|e| e.to_string())?; }
                sender.set_stream_delay_ms(Some(io.stream_delay_ms()));
                while let Some(frame) = io.capture().map_err(|e| e.to_string())? {
                    if let Some(packet) = sender.process(&frame).map_err(|e| e.to_string())? {
                        receiver.push("loopback", packet, now).map_err(|e| e.to_string())?;
                    }
                }
                let output = receiver.render(now).map_err(|e| e.to_string())?;
                io.playback(&output).map_err(|e| e.to_string())?;
                sender.render_reference(&output).map_err(|e| e.to_string())?;
                count += 1;
                if count % 10 == 0 {
                    let level = (output.iter().map(|s| (s[0]*s[0]+s[1]*s[1])*0.5).sum::<f32>() / 480.0).sqrt();
                    sink(CallEvent { session_id: id.clone(), event: json!({"type":"loopbackLevel","level":(level*5.0).min(1.0),"aec":sender.has_aec()}) });
                }
            }
            Ok(())
        }))
    }
    fn start<F>(
        &self,
        id: String,
        settings: Settings,
        loopback: bool,
        sink: EventSink,
        run: F,
    ) -> Result<(), CallError>
    where
        F: FnOnce(
                mpsc::Receiver<Command>,
                watch::Receiver<bool>,
                EventSink,
                String,
            )
                -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>>>>
            + Send
            + 'static,
    {
        // The future is deliberately !Send: DeviceIo and all CPAL streams stay on this thread.
        let (commands, rx) = mpsc::channel(64);
        let (stop, mut stopped) = watch::channel(false);
        let mut state = self.state.lock().unwrap();
        if loopback && state.active.is_some() {
            return Err(CallError::Invalid(
                "Leave the call before starting sound check",
            ));
        }
        if let Some(old) = state.active.take() {
            let _ = old.stop.send(true);
        }
        let shared = self.state.clone();
        let worker_id = id.clone();
        std::thread::Builder::new().name("pingo-call".into()).spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
            let result = match runtime {
                Ok(runtime) => runtime.block_on(async {
                    tokio::select! { result = run(rx, stopped.clone(), sink.clone(), worker_id.clone()) => result, _ = stopped.changed() => Ok(()) }
                }),
                Err(e) => Err(e.to_string()),
            };
            let mut state = shared.lock().unwrap();
            if state.active.as_ref().is_some_and(|a| a.id == worker_id) { state.active = None; }
            drop(state);
            if let Err(message) = result { sink(CallEvent { session_id: worker_id.clone(), event: json!({"type":"error", "message":message}) }); }
            sink(CallEvent { session_id: worker_id, event: json!({"type":"disconnected"}) });
        }).map_err(|_| CallError::Invalid("Could not create native call worker"))?;
        state.settings = settings;
        state.active = Some(Active {
            id,
            commands,
            stop,
            loopback,
        });
        Ok(())
    }
}
impl Drop for CallManager {
    fn drop(&mut self) {
        self.leave(None);
    }
}

pub fn rtc_url(origin: &str, path: &str) -> Result<String, CallError> {
    let base = url::Url::parse(origin).map_err(|_| CallError::Invalid("Invalid server origin"))?;
    let mut url = base
        .join(path)
        .map_err(|_| CallError::Invalid("Invalid RTC URL"))?;
    let scheme = match url.scheme() {
        "http" | "ws" => "ws",
        "https" | "wss" => "wss",
        _ => return Err(CallError::Invalid("RTC needs HTTP or WebSocket")),
    };
    url.set_scheme(scheme)
        .map_err(|_| CallError::Invalid("Invalid RTC scheme"))?;
    let secure = base.scheme() == "https";
    if url.host_str() != base.host_str()
        || url.port_or_known_default() != base.port_or_known_default()
        || (url.scheme() == "wss") != secure
        || url.path() != "/rtc"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(CallError::Invalid(
            "RTC admission URL must be /rtc on the configured server",
        ));
    }
    Ok(url.to_string())
}
async fn forward_events(
    mut events: tokio::sync::broadcast::Receiver<RtcEvent>,
    sink: EventSink,
    id: String,
    video_sink: Option<crate::video::PacketSink>,
) {
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                sink(CallEvent {
                    session_id: id.clone(),
                    event: json!({"type":"error", "message":"Native event stream fell behind; reconnecting"}),
                });
                return;
            }
            Err(_) => return,
        };
        let event = match event {
            RtcEvent::VideoFrame { track, frame } => {
                if let Some(sink) = &video_sink {
                    sink(crate::video::RemoteVideoFrame { track, frame });
                }
                continue;
            }
            RtcEvent::Joined {
                participant_id,
                participants,
            } => {
                json!({"type":"joined","participantId":participant_id,"participants":participants})
            }
            RtcEvent::ParticipantJoined(p) => json!({"type":"participantJoined","participant":p}),
            RtcEvent::ParticipantUpdated(p) => json!({"type":"participantUpdated","participant":p}),
            RtcEvent::ParticipantLeft(id) => json!({"type":"participantLeft","participantId":id}),
            RtcEvent::TrackPublished(track) => json!({"type":"trackPublished","track":track}),
            RtcEvent::TrackUnpublished {
                participant_id,
                mid,
            } => json!({"type":"trackUnpublished","participantId":participant_id,"mid":mid}),
            RtcEvent::Connected => json!({"type":"connected"}),
            RtcEvent::Disconnected => json!({"type":"disconnected"}),
            RtcEvent::Data(data) => json!({"type":"data","data":data}),
            RtcEvent::Quality(q) => {
                json!({"type":"quality","quality":q.quality.to_string(),"lossPercent":q.loss_percent,"jitterMs":q.jitter_ms,"rttMs":q.rtt_ms,"delayMs":q.delay_ms})
            }
            RtcEvent::DeviceError(message) => json!({"type":"deviceError","message":message}),
            RtcEvent::Error { code, message } => {
                json!({"type":"error","code":code,"message":message})
            }
            RtcEvent::PlayoutStats(stats) => {
                json!({"type":"playout","receivedPackets":stats.values().map(|s| s.received).sum::<u64>()})
            }
            _ => continue,
        };
        sink(CallEvent {
            session_id: id.clone(),
            event,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_cannot_send_tokens_to_another_origin() {
        assert_eq!(
            rtc_url("https://chat.example", "/rtc").unwrap(),
            "wss://chat.example/rtc"
        );
        for url in [
            "wss://evil.example/rtc",
            "ws://chat.example/rtc",
            "wss://chat.example/rtc?token=secret",
            "wss://chat.example/api",
        ] {
            assert!(rtc_url("https://chat.example", url).is_err());
        }
    }
    #[test]
    fn gates_and_studio_follow_native_pipeline_settings() {
        let mut s = Settings {
            push_to_talk: true,
            ..Default::default()
        };
        assert_eq!(s.audio().unwrap().gate, Gate::PushToTalk { pressed: false });
        s.ptt_pressed = true;
        assert_eq!(s.audio().unwrap().gate, Gate::PushToTalk { pressed: true });
        s.deafened = true;
        assert_eq!(s.audio().unwrap().gate, Gate::Muted);
        s.studio = true;
        let a = s.audio().unwrap();
        assert_eq!(a.profile, Profile::Studio);
        assert!(!a.echo_cancel && !a.agc);
    }
    #[test]
    fn rejects_invalid_and_nonfinite_mix() {
        let mut s = Settings::default();
        for volume in [f32::NAN, f32::INFINITY, -0.1, 2.0] {
            s.volumes.insert("peer".into(), Mix { volume, pan: 0.0 });
            assert!(s.audio().is_err());
        }
        s.volumes.clear();
        s.suppression = "Unknown".into();
        assert!(s.audio().is_err());
    }
    #[test]
    fn stale_leave_and_commands_do_not_change_replacement_session() {
        let manager = CallManager::default();
        let (commands, mut rx) = mpsc::channel(2);
        let (stop, _) = watch::channel(false);
        manager.state.lock().unwrap().active = Some(Active {
            id: "new".into(),
            commands,
            stop,
            loopback: false,
        });
        manager.leave(Some("old"));
        assert!(manager.state.lock().unwrap().active.is_some());
        assert!(matches!(
            manager.configure("old", Settings::default()),
            Err(CallError::Idle)
        ));
        manager.ptt(true).unwrap();
        assert!(matches!(rx.try_recv(), Ok(Command::Settings(_))));
        manager.leave(Some("new"));
        assert!(manager.state.lock().unwrap().active.is_none());
    }
}
