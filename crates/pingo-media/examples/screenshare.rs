//! Portal capture or a deterministic native publisher for browser interop tests.
use pingo_media::{
    rtc::{Client, ClientConfig, Command, RtcEvent},
    video::{
        capture::{CaptureOptions, ScreenCapturer, synthetic::SyntheticScreen},
        encode::{VideoCodec, VideoMode},
        simulcast::{VideoPublishConfig, VideoSource},
    },
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

// Cancelling this future must also release blocking workers before Tokio shuts down.
struct StopWorkers(Arc<AtomicBool>);
impl Drop for StopWorkers {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
use tokio::sync::{broadcast, mpsc};
fn arg(name: &str) -> Option<String> {
    std::env::args()
        .collect::<Vec<_>>()
        .windows(2)
        .find(|p| p[0] == name)
        .map(|p| p[1].clone())
}
fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if flag("--help") {
        println!(
            "screenshare --url ws://host/rtc --token TOKEN [--synthetic] [--motion] [--fps N] [--vp8] [--software] [--seconds N] [--source monitor|window] [--restore-token-file PATH] [--audio-app SERIAL]\n--capture-only runs capture and encode without a room. --synthetic accepts --width/--height (default 640x360). --probe reports capture nodes. Tokens may use PINGO_MEDIA_TOKEN."
        );
        return Ok(());
    }
    #[cfg(all(target_os = "linux", feature = "linux-capture"))]
    if flag("--probe") {
        for node in pingo_media::video::capture::linux::nodes()? {
            println!("{}: {} ({})", node.serial, node.name, node.media_class);
        }
        return Ok(());
    }
    let mode = if flag("--motion") {
        VideoMode::Motion
    } else {
        VideoMode::Text
    };
    let seconds = arg("--seconds")
        .map(|s| s.parse::<u64>())
        .transpose()?
        .unwrap_or(20);
    let synthetic = flag("--synthetic");
    let fps = arg("--fps")
        .map(|n| n.parse::<u32>())
        .transpose()?
        .unwrap_or(mode.fps());
    let mut capture: Box<dyn ScreenCapturer> = if synthetic {
        let width = arg("--width")
            .map(|n| n.parse::<u32>())
            .transpose()?
            .unwrap_or(640);
        let height = arg("--height")
            .map(|n| n.parse::<u32>())
            .transpose()?
            .unwrap_or(360);
        Box::new(SyntheticScreen::new(
            width,
            height,
            mode == VideoMode::Motion,
        )?)
    } else {
        #[cfg(all(target_os = "linux", feature = "linux-capture"))]
        {
            Box::new(pingo_media::video::capture::linux::PortalScreen::default())
        }
        #[cfg(not(all(target_os = "linux", feature = "linux-capture")))]
        {
            anyhow::bail!("enable linux-capture or use --synthetic");
        }
    };
    let token_file = arg("--restore-token-file");
    let options = CaptureOptions {
        max_fps: fps,
        source: arg("--source").unwrap_or_else(|| "monitor".into()),
        restore_token: token_file
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok()),
        ..Default::default()
    };
    match tokio::time::timeout(Duration::from_secs(45), capture.start(options)).await {
        Ok(Ok(())) => {}
        result => {
            // Explicitly close a pending portal request before runtime shutdown.
            capture.stop().await?;
            result??;
        }
    }
    if let Some(path) = token_file
        && let Some(token) = capture.restore_token()
    {
        std::fs::write(path, token)?;
    }
    // next_frame blocks only a capture worker, never a tokio/RTC execution thread.
    let first = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let frame = capture
            .next_frame(Duration::from_secs(5))?
            .ok_or_else(|| anyhow::anyhow!("no capture frame in five seconds"))?;
        Ok((capture, frame))
    })
    .await??;
    let (mut capture, first) = first;
    let (width, height) = mode.max_size();
    let scale = (width as f64 / first.width as f64)
        .min(height as f64 / first.height as f64)
        .min(1.0);
    let mut video = VideoPublishConfig::screen(
        ((first.width as f64 * scale) as u32 & !1).max(2),
        ((first.height as f64 * scale) as u32 & !1).max(2),
        mode,
    );
    video.prefer_hardware = !flag("--software");
    video.fps = fps;
    video.layers()?;
    if flag("--vp8") {
        video.codec = VideoCodec::Vp8;
    }
    if flag("--capture-only") {
        let config = video.clone();
        let (mut capture, result) = tokio::task::spawn_blocking(move || {
            let result = measure(capture.as_mut(), first, config, seconds, synthetic);
            (capture, result)
        })
        .await?;
        capture.stop().await?;
        return result;
    }
    let url = arg("--url").ok_or_else(|| anyhow::anyhow!("--url required"))?;
    let token = arg("--token")
        .or_else(|| std::env::var("PINGO_MEDIA_TOKEN").ok())
        .ok_or_else(|| anyhow::anyhow!("--token or PINGO_MEDIA_TOKEN required"))?;
    let client = Client::connect(ClientConfig::new(url, token)).await?;
    let mut events = client.events.subscribe();
    let (commands, rx) = mpsc::channel(16);
    commands.send(Command::PublishAudio(false)).await?;
    commands
        .send(Command::PublishVideo {
            source: VideoSource::Screen,
            config: Some(video.clone()),
        })
        .await?;
    let stop = Arc::new(AtomicBool::new(false));
    let _stop_workers = StopWorkers(stop.clone());
    let stopped = stop.clone();
    let producer = commands.clone();
    let runtime = tokio::runtime::Handle::current();
    let capture_task = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let mut next = Some(first);
        let start = Instant::now();
        let mut frames = 0u64;
        let result = (|| -> anyhow::Result<()> {
            while !stopped.load(Ordering::Relaxed) {
                if synthetic {
                    let due = start + Duration::from_secs_f64(frames as f64 / video.fps as f64);
                    std::thread::sleep(due.saturating_duration_since(Instant::now()));
                }
                let frame = match next.take() {
                    Some(frame) => Some(frame),
                    None => capture.next_frame(Duration::from_millis(100))?,
                };
                if let Some(frame) = frame {
                    frames += 1;
                    let result = producer.try_send(Command::VideoFrame {
                        source: VideoSource::Screen,
                        frame: Box::new(frame),
                    });
                    if matches!(result, Err(mpsc::error::TrySendError::Closed(_))) {
                        break;
                    }
                }
            }
            Ok(())
        })();
        eprintln!(
            "capture {}x{}, {frames} frames / {:.2}s = {:.2} fps",
            video.width,
            video.height,
            start.elapsed().as_secs_f64(),
            frames as f64 / start.elapsed().as_secs_f64()
        );
        runtime.block_on(capture.stop())?;
        result
    });
    #[cfg(all(target_os = "linux", feature = "linux-capture"))]
    let audio_task = if let Some(serial) = arg("--audio-app") {
        let tx = commands.clone();
        let stopped = stop.clone();
        commands.send(Command::PublishScreenAudio(true)).await?;
        Some(tokio::task::spawn_blocking(
            move || -> anyhow::Result<()> {
                let mut capture = pingo_media::video::capture::linux::AppAudioCapturer::default();
                capture.start(&serial)?;
                while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Some(frame) = capture.next_frame(Duration::from_millis(20))? {
                        let _ = tx.try_send(Command::CaptureScreenAudio(Box::new(frame)));
                    }
                }
                Ok(())
            },
        ))
    } else {
        None
    };
    let run = client.run(None, rx);
    tokio::pin!(run);
    let deadline = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(deadline);
    let mut leaving = false;
    let result = loop {
        tokio::select! {
            result=&mut run=>break result.map_err(anyhow::Error::from),
            event=events.recv()=>match event {
                Ok(RtcEvent::Connected)=>println!("native publisher connected"),
                Ok(RtcEvent::Negotiated { .. })=>println!("native tracks negotiated"),
                Ok(RtcEvent::Error { code,message })=>break Err(anyhow::anyhow!("{code}: {message}")),
                Ok(RtcEvent::DeviceError(error))=>break Err(anyhow::anyhow!(error)),
                Err(broadcast::error::RecvError::Closed)=>break Ok(()),
                _=>{}
            },
            _=&mut deadline,if !leaving=>{ leaving=true;commands.send(Command::Leave).await?; }
            _=tokio::signal::ctrl_c(),if !leaving=>{ leaving=true;commands.send(Command::Leave).await?; }
        }
    };
    stop.store(true, Ordering::Relaxed);
    capture_task.await??;
    #[cfg(all(target_os = "linux", feature = "linux-capture"))]
    if let Some(task) = audio_task {
        task.await??;
    }
    result
}
fn measure(
    capture: &mut dyn ScreenCapturer,
    first: pingo_media::video::capture::CapturedFrame,
    config: VideoPublishConfig,
    seconds: u64,
    synthetic: bool,
) -> anyhow::Result<()> {
    use pingo_media::video::simulcast::SimulcastEncoder;
    let mut encoder = SimulcastEncoder::new(config.clone())?;
    println!(
        "capture {}x{} -> {}x{}, mode {:?}, encoders {:?}",
        first.width,
        first.height,
        config.width,
        config.height,
        config.mode,
        encoder.backends()
    );
    let start = Instant::now();
    let mut encode = Duration::ZERO;
    let mut frames = 0;
    let mut encoded = 0;
    let mut first = Some(first);
    while start.elapsed() < Duration::from_secs(seconds) {
        if synthetic {
            let due = start + Duration::from_secs_f64(frames as f64 / config.fps as f64);
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
        }
        let frame = match first.take() {
            Some(frame) => Some(frame),
            None => capture.next_frame(Duration::from_millis(100))?,
        };
        if let Some(frame) = frame {
            frames += 1;
            let now = Instant::now();
            let packets = encoder.encode(&frame)?;
            encode += now.elapsed();
            if !packets.is_empty() {
                encoded += 1;
            }
        }
    }
    println!(
        "{frames} captured / {encoded} encoded source frames in {:.2}s: {:.2} capture fps, {:.2} encode fps, {:.2} ms/source-frame (all layers + conversion/scaling)",
        start.elapsed().as_secs_f64(),
        frames as f64 / start.elapsed().as_secs_f64(),
        encoded as f64 / start.elapsed().as_secs_f64(),
        encode.as_secs_f64() * 1000.0 / encoded.max(1) as f64
    );
    Ok(())
}
