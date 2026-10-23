//! A bounded producer/consumer boundary keeps codec work off the RTC/audio clock.
use super::{
    Result, VideoError,
    capture::CapturedFrame,
    encode::EncodedVideoFrame,
    simulcast::{SimulcastEncoder, VideoPublishConfig},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

pub(crate) struct EncoderWorker {
    input: Option<mpsc::SyncSender<Box<CapturedFrame>>>,
    output: tokio::sync::mpsc::Receiver<Result<EncodedVideoFrame>>,
    keyframe: Arc<AtomicU8>,
    bandwidth: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl EncoderWorker {
    pub fn new(config: VideoPublishConfig) -> Result<Self> {
        config.layers()?;
        let (input, frames) = mpsc::sync_channel::<Box<CapturedFrame>>(2);
        let (output, encoded) = tokio::sync::mpsc::channel(6);
        let keyframe = Arc::new(AtomicU8::new(7));
        let keys = keyframe.clone();
        let bandwidth = Arc::new(AtomicU64::new(0));
        let budget = bandwidth.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("pingo-video-encoder".into())
            .spawn(move || {
                let mut encoder = match SimulcastEncoder::new(config) {
                    Ok(encoder) => {
                        let _ = ready_tx.send(Ok(()));
                        encoder
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                while !stopped.load(Ordering::Relaxed) {
                    let mut frame = match frames.recv_timeout(Duration::from_millis(20)) {
                        Ok(frame) => frame,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(_) => break,
                    };
                    while let Ok(newer) = frames.try_recv() {
                        frame = newer;
                    }
                    let keymask = keys.swap(0, Ordering::Relaxed);
                    for (bit, rid) in [(1, "l"), (2, "m"), (4, "h")] {
                        if keymask & bit != 0 {
                            encoder.request_keyframe(Some(rid));
                        }
                    }
                    let bandwidth = budget.swap(0, Ordering::Relaxed);
                    if bandwidth > 0
                        && let Err(error) = encoder.set_bandwidth(bandwidth)
                    {
                        let _ = output.try_send(Err(error));
                    }
                    match encoder.encode(&frame) {
                        Ok(packets) => {
                            for packet in packets {
                                if output.try_send(Ok(packet)).is_err() {
                                    keys.fetch_or(7, Ordering::Relaxed);
                                }
                            }
                        }
                        Err(error) => {
                            let _ = output.try_send(Err(error));
                        }
                    }
                }
            })
            .map_err(|e| VideoError::Encoder(e.to_string()))?;
        let worker = Self {
            input: Some(input),
            output: encoded,
            keyframe,
            bandwidth,
            stop,
            thread: Some(thread),
        };
        ready
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| VideoError::Encoder(e.to_string()))??;
        Ok(worker)
    }
    pub fn frame(&self, frame: Box<CapturedFrame>) -> bool {
        self.input
            .as_ref()
            .is_some_and(|input| input.try_send(frame).is_ok())
    }
    pub fn keyframe(&self, rid: Option<&str>) {
        let bits = match rid {
            Some("l") => 1,
            Some("m") => 2,
            Some("h") => 4,
            _ => 7,
        };
        self.keyframe.fetch_or(bits, Ordering::Relaxed);
    }
    pub fn bandwidth(&self, bits_per_second: u64) {
        self.bandwidth.store(bits_per_second, Ordering::Relaxed);
    }
    pub fn next(&mut self) -> Option<Result<EncodedVideoFrame>> {
        self.output.try_recv().ok()
    }
}
impl Drop for EncoderWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.input.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
