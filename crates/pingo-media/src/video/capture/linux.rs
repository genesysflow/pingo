//! Portal permission stays alive for the lifetime of the PipeWire stream.
//! We negotiate CPU-mapped packed pixels only. DMA-BUF import is deliberately
//! not advertised: its GPU synchronization/modifier contract needs an importer.
use super::*;
use ashpd::desktop::{
    PersistMode,
    screencast::{Screencast, SelectSourcesOptions, SourceType},
};
use pipewire as pw;
use pw::{properties::properties, spa};
use std::{
    cell::{Cell, RefCell},
    os::fd::OwnedFd,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Instant,
};

fn capture_error(e: impl std::fmt::Display) -> VideoError {
    VideoError::Capture(e.to_string())
}

struct Worker<T> {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    frames: mpsc::Receiver<Result<T>>,
}
impl<T> Worker<T> {
    fn next(&self, timeout: Duration) -> Result<Option<T>> {
        match self.frames.recv_timeout(timeout) {
            Ok(value) => {
                let mut latest = value;
                while let Ok(value) = self.frames.try_recv() {
                    latest = value;
                }
                latest.map(Some)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err(VideoError::Capture("PipeWire stream stopped".into())),
        }
    }
}
impl<T> Drop for Worker<T> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Default)]
pub struct PortalScreen {
    worker: Option<Worker<CapturedFrame>>,
    session: Option<ashpd::desktop::Session<Screencast>>,
    token: Option<String>,
}
impl ScreenCapturer for PortalScreen {
    fn list_sources(&self) -> CaptureFuture<'_, Vec<Source>> {
        Box::pin(async {
            let portal = Screencast::new().await.map_err(capture_error)?;
            let types = portal
                .available_source_types()
                .await
                .map_err(capture_error)?;
            Ok([
                (SourceType::Monitor, "monitor", "Choose a monitor…"),
                (SourceType::Window, "window", "Choose a window…"),
            ]
            .into_iter()
            .filter(|(kind, _, _)| types.contains(*kind))
            .map(|(_, id, name)| Source {
                id: id.into(),
                name: name.into(),
            })
            .collect())
        })
    }
    fn start(&mut self, options: CaptureOptions) -> CaptureFuture<'_, ()> {
        Box::pin(async move {
            self.stop().await?;
            if !(1..=60).contains(&options.max_fps) {
                return Err(VideoError::Invalid("fps must be 1..60"));
            }
            let source = match options.source.as_str() {
                "monitor" => SourceType::Monitor,
                "window" => SourceType::Window,
                _ => {
                    return Err(VideoError::Invalid(
                        "portal source must be monitor or window",
                    ));
                }
            };
            let portal = Screencast::new().await.map_err(capture_error)?;
            let session = portal
                .create_session(Default::default())
                .await
                .map_err(capture_error)?;
            let result = async {
                let cursor = match options.cursor {
                    CursorMode::Hidden => ashpd::desktop::screencast::CursorMode::Hidden,
                    CursorMode::Embedded => ashpd::desktop::screencast::CursorMode::Embedded,
                };
                let modes = portal
                    .available_cursor_modes()
                    .await
                    .map_err(capture_error)?;
                if !modes.contains(cursor) {
                    return Err(VideoError::Unsupported("portal cursor mode"));
                }
                portal
                    .select_sources(
                        &session,
                        SelectSourcesOptions::default()
                            .set_sources(Some(source.into()))
                            .set_multiple(false)
                            .set_cursor_mode(cursor)
                            .set_persist_mode(PersistMode::ExplicitlyRevoked)
                            .set_restore_token(
                                options.restore_token.as_deref().or(self.token.as_deref()),
                            ),
                    )
                    .await
                    .map_err(capture_error)?
                    .response()
                    .map_err(capture_error)?;
                let response = portal
                    .start(&session, None, Default::default())
                    .await
                    .map_err(capture_error)?
                    .response()
                    .map_err(capture_error)?;
                let node = response
                    .streams()
                    .first()
                    .ok_or(VideoError::Capture("portal returned no stream".into()))?
                    .pipe_wire_node_id();
                let fd = portal
                    .open_pipe_wire_remote(&session, Default::default())
                    .await
                    .map_err(capture_error)?;
                let worker = video_worker(Some(fd), node, options.max_fps)?;
                Ok((worker, response.restore_token().map(str::to_owned)))
            }
            .await;
            match result {
                Ok((worker, token)) => {
                    self.worker = Some(worker);
                    self.token = token;
                    self.session = Some(session);
                    Ok(())
                }
                Err(error) => {
                    let _ = session.close().await;
                    Err(error)
                }
            }
        })
    }
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<CapturedFrame>> {
        self.worker
            .as_ref()
            .map_or(Ok(None), |worker| worker.next(timeout))
    }
    fn stop(&mut self) -> CaptureFuture<'_, ()> {
        Box::pin(async {
            self.worker.take();
            if let Some(session) = self.session.take() {
                session.close().await.map_err(capture_error)?;
            }
            Ok(())
        })
    }
    fn restore_token(&self) -> Option<&str> {
        self.token.as_deref()
    }
}
impl Drop for PortalScreen {
    fn drop(&mut self) {
        self.worker.take();
        if let Some(session) = self.session.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(async move {
                let _ = session.close().await;
            });
        }
    }
}

#[derive(Clone, Debug)]
pub struct PipeWireNode {
    pub id: u32,
    pub serial: String,
    pub name: String,
    pub media_class: String,
    pub process_id: Option<u32>,
    pub application: String,
}
/// The round trip is bounded so a dead PipeWire daemon cannot hang a picker.
pub fn nodes() -> Result<Vec<PipeWireNode>> {
    pw::init();
    let main = pw::main_loop::MainLoopRc::new(None).map_err(capture_error)?;
    let context = pw::context::ContextRc::new(&main, None).map_err(capture_error)?;
    let core = context.connect_rc(None).map_err(capture_error)?;
    let registry = core.get_registry().map_err(capture_error)?;
    let nodes = Rc::new(RefCell::new(Vec::new()));
    let found = nodes.clone();
    let _registry = registry
        .add_listener_local()
        .global(move |global| {
            if global.type_ != pw::types::ObjectType::Node {
                return;
            }
            let Some(props) = global.props else {
                return;
            };
            found.borrow_mut().push(PipeWireNode {
                id: global.id,
                serial: props.get("object.serial").unwrap_or("").into(),
                name: props
                    .get("node.description")
                    .or_else(|| props.get("node.name"))
                    .unwrap_or("Unnamed")
                    .into(),
                media_class: props.get("media.class").unwrap_or("").into(),
                process_id: props
                    .get("application.process.id")
                    .and_then(|v| v.parse().ok()),
                application: props.get("application.name").unwrap_or("").into(),
            });
        })
        .register();
    let done = Rc::new(Cell::new(false));
    let done_cb = done.clone();
    let seq = core.sync(0).map_err(capture_error)?;
    let _listener = core
        .add_listener_local()
        .done(move |id, s| {
            if id == pw::core::PW_ID_CORE && seq == s {
                done_cb.set(true);
            }
        })
        .register();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !done.get() {
        if Instant::now() >= deadline {
            return Err(VideoError::Capture("PipeWire enumeration timed out".into()));
        }
        main.loop_()
            .iterate(pw::loop_::Timeout::Finite(Duration::from_millis(20)));
    }
    Ok(nodes.borrow().clone())
}

#[derive(Default)]
pub struct PipeWireCamera {
    worker: Option<Worker<CapturedFrame>>,
}
impl CameraCapturer for PipeWireCamera {
    fn list_sources(&self) -> CaptureFuture<'_, Vec<Source>> {
        Box::pin(async {
            tokio::task::spawn_blocking(|| {
                Ok(nodes()?
                    .into_iter()
                    .filter(|n| n.media_class == "Video/Source")
                    .map(|n| Source {
                        id: n.serial,
                        name: n.name,
                    })
                    .collect())
            })
            .await
            .map_err(capture_error)?
        })
    }
    fn start(&mut self, options: CaptureOptions) -> CaptureFuture<'_, ()> {
        Box::pin(async move {
            self.worker.take();
            let node = nodes()?
                .into_iter()
                .find(|n| n.serial == options.source && n.media_class == "Video/Source")
                .ok_or(VideoError::Invalid("unknown camera serial"))?;
            if !(1..=60).contains(&options.max_fps) {
                return Err(VideoError::Invalid("fps must be 1..60"));
            }
            self.worker = Some(video_worker(None, node.id, options.max_fps)?);
            Ok(())
        })
    }
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<CapturedFrame>> {
        self.worker
            .as_ref()
            .map_or(Ok(None), |worker| worker.next(timeout))
    }
    fn stop(&mut self) -> CaptureFuture<'_, ()> {
        Box::pin(async {
            self.worker.take();
            Ok(())
        })
    }
}

fn serialize(object: spa::pod::Object) -> Result<Vec<u8>> {
    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(object),
    )
    .map(|(cursor, _)| cursor.into_inner())
    .map_err(capture_error)
}
fn video_worker(fd: Option<OwnedFd>, node: u32, fps: u32) -> Result<Worker<CapturedFrame>> {
    let (tx, rx) = mpsc::sync_channel(2);
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let (ready_tx, ready) = mpsc::sync_channel(1);
    let thread = thread::Builder::new()
        .name("pingo-video-capture".into())
        .spawn(move || {
            let result = run_video(fd, node, fps, &stopped, tx.clone(), ready_tx);
            if let Err(error) = result {
                let _ = tx.try_send(Err(error));
            }
        })
        .map_err(capture_error)?;
    let worker = Worker {
        stop,
        thread: Some(thread),
        frames: rx,
    };
    ready
        .recv_timeout(Duration::from_secs(3))
        .map_err(capture_error)??;
    Ok(worker)
}
fn run_video(
    fd: Option<OwnedFd>,
    node: u32,
    fps: u32,
    stop: &AtomicBool,
    tx: mpsc::SyncSender<Result<CapturedFrame>>,
    ready: mpsc::SyncSender<Result<()>>,
) -> Result<()> {
    pw::init();
    let main = pw::main_loop::MainLoopRc::new(None).map_err(capture_error)?;
    let context = pw::context::ContextRc::new(&main, None).map_err(capture_error)?;
    let core = match fd {
        Some(fd) => context.connect_fd_rc(fd, None),
        None => context.connect_rc(None),
    }
    .map_err(capture_error)?;
    let stream = pw::stream::StreamBox::new(
        &core,
        "Pingo video capture",
        properties! {
            "media.type" => "Video", "media.category" => "Capture", "media.role" => "Screen",
            "node.dont-reconnect" => "true",
        },
    )
    .map_err(capture_error)?;
    let epoch = Instant::now();
    let mut first_pts = None;
    let errors = tx.clone();
    let _listener = stream
        .add_local_listener_with_user_data(spa::param::video::VideoInfoRaw::default())
        .state_changed(move |_, _, _, state| {
            if let pw::stream::StreamState::Error(error) = state {
                let _ = errors.try_send(Err(capture_error(error)));
            }
        })
        .param_changed(|_, info, id, param| {
            if id == spa::param::ParamType::Format.as_raw()
                && let Some(param) = param
            {
                let _ = info.parse(param);
            }
        })
        .process(move |stream, info| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let timestamp = buffer
                .find_meta::<spa::buffer::meta::MetaHeader>()
                .filter(|h| h.pts() >= 0)
                .map(|h| {
                    let base = *first_pts.get_or_insert(h.pts());
                    Duration::from_nanos(h.pts().saturating_sub(base).max(0) as u64)
                })
                .unwrap_or_else(|| epoch.elapsed());
            let damage = buffer
                .find_meta::<spa::buffer::meta::MetaVideoDamage>()
                .map(|meta| {
                    meta.iter()
                        .map(|r| DamageRect {
                            x: r.position().x.max(0) as u32,
                            y: r.position().y.max(0) as u32,
                            width: r.size().width,
                            height: r.size().height,
                        })
                        .collect()
                });
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let width = info.size().width;
            let height = info.size().height;
            if width == 0 || height == 0 || width > 8192 || height > 8192 {
                return;
            }
            let offset = data.chunk().offset() as usize;
            let stride = data.chunk().stride();
            if stride < width as i32 * 4 {
                return;
            }
            let len = stride as usize * height as usize;
            let Some(bytes) = data.data() else {
                return;
            };
            let Some(bytes) = bytes.get(offset..offset.saturating_add(len)) else {
                return;
            };
            let format = if info.format() == spa::param::video::VideoFormat::BGRx {
                PixelFormat::Bgrx
            } else {
                PixelFormat::Rgbx
            };
            let frame = CapturedFrame {
                width,
                height,
                stride: stride as usize,
                format,
                data: bytes.to_vec(),
                timestamp,
                damage,
            };
            let _ = tx.try_send(Ok(frame));
        })
        .register()
        .map_err(capture_error)?;
    let object = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaType,
            Id,
            spa::param::format::MediaType::Video
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaSubtype,
            Id,
            spa::param::format::MediaSubtype::Raw
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            spa::param::video::VideoFormat::BGRx,
            spa::param::video::VideoFormat::BGRx,
            spa::param::video::VideoFormat::RGBx
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            spa::utils::Rectangle {
                width: 1920,
                height: 1080
            },
            spa::utils::Rectangle {
                width: 2,
                height: 2
            },
            spa::utils::Rectangle {
                width: 8192,
                height: 8192
            }
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            spa::utils::Fraction { num: fps, denom: 1 },
            spa::utils::Fraction { num: 0, denom: 1 },
            spa::utils::Fraction { num: fps, denom: 1 }
        )
    );
    let bytes = serialize(object)?;
    let mut params =
        [spa::pod::Pod::from_bytes(&bytes).ok_or(VideoError::Invalid("video format pod"))?];
    stream
        .connect(
            spa::utils::Direction::Input,
            Some(node),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(capture_error)?;
    let _ = ready.send(Ok(()));
    while !stop.load(Ordering::Relaxed) {
        main.loop_()
            .iterate(pw::loop_::Timeout::Finite(Duration::from_millis(20)));
    }
    Ok(())
}

/// Capture one application's playback ports, never the mixed output monitor.
/// A serial is resolved again at start to avoid selecting a recycled node id.
#[derive(Default)]
pub struct AppAudioCapturer {
    worker: Option<Worker<crate::StereoFrame>>,
}
impl AppAudioCapturer {
    pub fn list_sources() -> Result<Vec<PipeWireNode>> {
        Ok(nodes()?.into_iter().filter(shareable_audio).collect())
    }
    pub fn start(&mut self, serial: &str) -> Result<()> {
        self.worker.take();
        let node = Self::list_sources()?
            .into_iter()
            .find(|node| node.serial == serial)
            .ok_or(VideoError::Invalid(
                "audio source must be another application's playback stream",
            ))?;
        let (tx, rx) = mpsc::sync_channel(10);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("pingo-share-audio".into())
            .spawn(move || {
                if let Err(error) = run_audio(&node.serial, &stopped, tx.clone(), ready_tx) {
                    let _ = tx.try_send(Err(error));
                }
            })
            .map_err(capture_error)?;
        let worker = Worker {
            stop,
            thread: Some(thread),
            frames: rx,
        };
        ready
            .recv_timeout(Duration::from_secs(3))
            .map_err(capture_error)??;
        self.worker = Some(worker);
        Ok(())
    }
    pub fn next_frame(&mut self, timeout: Duration) -> Result<Option<crate::StereoFrame>> {
        // Audio must preserve every 10 ms quantum, unlike latest-only video.
        let Some(worker) = &self.worker else {
            return Ok(None);
        };
        match worker.frames.recv_timeout(timeout) {
            Ok(frame) => frame.map(Some),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err(VideoError::Capture("app audio stream stopped".into())),
        }
    }
    pub fn stop(&mut self) {
        self.worker.take();
    }
}
fn shareable_audio(node: &PipeWireNode) -> bool {
    node.media_class == "Stream/Output/Audio"
        && node.process_id.is_some_and(|pid| pid != std::process::id())
        && !node.application.to_ascii_lowercase().contains("pingo")
        && !node.name.to_ascii_lowercase().contains("pingo")
}
fn run_audio(
    serial: &str,
    stop: &AtomicBool,
    tx: mpsc::SyncSender<Result<crate::StereoFrame>>,
    ready: mpsc::SyncSender<Result<()>>,
) -> Result<()> {
    pw::init();
    let main = pw::main_loop::MainLoopRc::new(None).map_err(capture_error)?;
    let context = pw::context::ContextRc::new(&main, None).map_err(capture_error)?;
    let core = context.connect_rc(None).map_err(capture_error)?;
    let stream = pw::stream::StreamBox::new(&core, "Pingo app audio capture", properties! {
        "media.type" => "Audio", "media.category" => "Capture", "media.role" => "Music",
        "target.object" => serial, "stream.capture.sink" => "true", "node.dont-reconnect" => "true",
        "audio.rate" => "48000", "audio.channels" => "2", "audio.position" => "[ FL FR ]",
    }).map_err(capture_error)?;
    let mut partial = Vec::<[f32; 2]>::with_capacity(crate::FRAME_SAMPLES);
    let errors = tx.clone();
    let _listener = stream
        .add_local_listener_with_user_data(())
        .state_changed(move |_, _, _, state| {
            if let pw::stream::StreamState::Error(error) = state {
                let _ = errors.try_send(Err(capture_error(error)));
            }
        })
        .process(move |stream, _| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let offset = data.chunk().offset() as usize;
            let size = data.chunk().size() as usize;
            let Some(bytes) = data.data() else {
                return;
            };
            let Some(bytes) = bytes.get(offset..offset.saturating_add(size)) else {
                return;
            };
            for stereo in bytes.chunks_exact(8) {
                partial.push([
                    f32::from_le_bytes(stereo[..4].try_into().unwrap()),
                    f32::from_le_bytes(stereo[4..].try_into().unwrap()),
                ]);
                if partial.len() == crate::FRAME_SAMPLES {
                    let frame = std::array::from_fn(|i| partial[i]);
                    let _ = tx.try_send(Ok(frame));
                    partial.clear();
                }
            }
        })
        .register()
        .map_err(capture_error)?;
    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_rate(48_000);
    info.set_channels(2);
    let bytes = serialize(spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: info.into(),
    })?;
    let mut params =
        [spa::pod::Pod::from_bytes(&bytes).ok_or(VideoError::Invalid("audio format pod"))?];
    stream
        .connect(
            spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(capture_error)?;
    let _ = ready.send(Ok(()));
    while !stop.load(Ordering::Relaxed) {
        main.loop_()
            .iterate(pw::loop_::Timeout::Finite(Duration::from_millis(20)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn app_audio_rejects_call_output_and_unknown_owners() {
        let mut node = PipeWireNode {
            id: 1,
            serial: "1".into(),
            name: "Music".into(),
            media_class: "Stream/Output/Audio".into(),
            process_id: Some(std::process::id()),
            application: "Player".into(),
        };
        assert!(!shareable_audio(&node));
        node.process_id = None;
        assert!(!shareable_audio(&node));
        node.process_id = Some(std::process::id().wrapping_add(1));
        assert!(shareable_audio(&node));
        node.application = "Pingo".into();
        assert!(!shareable_audio(&node));
        node.application = "Player".into();
        node.media_class = "Audio/Sink".into();
        assert!(!shareable_audio(&node));
    }
}
