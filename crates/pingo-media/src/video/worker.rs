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
    time::{Duration, Instant},
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
                let mut latest = None::<Box<CapturedFrame>>;
                let mut captured_at = Instant::now();
                let mut capture_time = Duration::ZERO;
                while !stopped.load(Ordering::Relaxed) {
                    let mut incoming = match frames.recv_timeout(Duration::from_millis(20)) {
                        Ok(frame) => Some(frame),
                        Err(mpsc::RecvTimeoutError::Timeout) => None,
                        Err(_) => break,
                    };
                    while let Ok(newer) = frames.try_recv() {
                        incoming = Some(newer);
                    }
                    let keymask = keys.swap(0, Ordering::Relaxed);
                    if let Some(frame) = incoming {
                        capture_time = frame.timestamp;
                        captured_at = Instant::now();
                        latest = Some(frame);
                    } else if keymask == 0 {
                        continue;
                    } else if let Some(frame) = &mut latest {
                        // Static portal sources may send no new buffers. A PLI
                        // still needs a fresh IDR, with an advancing RTP clock.
                        frame.timestamp = capture_time.saturating_add(captured_at.elapsed());
                    }
                    let Some(frame) = &latest else {
                        keys.fetch_or(keymask, Ordering::Relaxed);
                        continue;
                    };
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
                    match encoder.encode(frame) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::{capture::synthetic::SyntheticScreen, encode::VideoMode};

    fn packets(worker: &mut EncoderWorker) -> Vec<EncodedVideoFrame> {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut packets = Vec::new();
        while packets.len() < 2 && Instant::now() < deadline {
            if let Some(packet) = worker.next() {
                packets.push(packet.unwrap());
            } else {
                thread::sleep(Duration::from_millis(5));
            }
        }
        assert_eq!(packets.len(), 2, "both screen layers must be emitted");
        packets
    }

    #[test]
    fn pli_refreshes_a_static_screen_without_another_capture_buffer() {
        let mut config = VideoPublishConfig::screen(320, 180, VideoMode::Text);
        config.prefer_hardware = false;
        let mut worker = EncoderWorker::new(config).unwrap();
        let source = SyntheticScreen::new(320, 180, false).unwrap();
        assert!(worker.frame(Box::new(source.frame_at(0))));
        let first = packets(&mut worker);
        assert!(first.iter().all(|packet| packet.keyframe));
        worker.keyframe(Some("h"));
        let refreshed = packets(&mut worker);
        let high = refreshed
            .iter()
            .find(|p| p.rid.as_deref() == Some("h"))
            .unwrap();
        assert!(high.keyframe);
        assert!(high.timestamp > first[0].timestamp);
        let mut decoder = openh264::decoder::Decoder::new().unwrap();
        assert!(decoder.decode(&high.data).unwrap().is_some());
    }
}
