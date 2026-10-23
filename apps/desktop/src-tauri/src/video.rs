//! Linux fallback: FFmpeg decodes on bounded workers; only displayed images cross IPC.
use pingo_media::rtc::RemoteVideoPacket;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use str0m::unversioned::{Depacketizer, H264Depacketizer, Vp8Depacketizer};
use tauri::ipc::{Channel, Response};

pub type PacketSink = Arc<dyn Fn(RemoteVideoPacket) + Send + Sync>;
struct Surface {
    generation: u64,
    channel: Channel<Response>,
    in_flight: Option<u32>,
    next: u32,
    decoder: Option<mpsc::SyncSender<RemoteVideoPacket>>,
}
#[derive(Default)]
pub struct VideoHub {
    surfaces: Mutex<HashMap<String, Surface>>,
    next_surface: AtomicU64,
}
impl VideoHub {
    pub fn subscribe(&self, key: String, channel: Channel<Response>) -> Result<(), String> {
        let mut surfaces = self.surfaces.lock().unwrap();
        if surfaces.len() >= 16 && !surfaces.contains_key(&key) {
            return Err("Too many native video surfaces".into());
        }
        surfaces.insert(
            key,
            Surface {
                generation: self.next_surface.fetch_add(1, Ordering::Relaxed),
                channel,
                in_flight: None,
                next: 0,
                decoder: None,
            },
        );
        Ok(())
    }
    pub fn unsubscribe(&self, key: &str) {
        self.surfaces.lock().unwrap().remove(key);
    }
    pub fn clear_session(&self, session: &str) {
        self.surfaces
            .lock()
            .unwrap()
            .retain(|key, _| !key.starts_with(&format!("{session}:")));
    }
    pub fn ack(&self, key: &str, id: u32) {
        if let Some(surface) = self.surfaces.lock().unwrap().get_mut(key)
            && surface.in_flight == Some(id)
        {
            surface.in_flight = None;
        }
    }
    pub fn raw(&self, key: &str, width: u32, height: u32, pixels: &[u8], sent_ms: f64) -> bool {
        if width == 0
            || height == 0
            || width > 3840
            || height > 2160
            || pixels.len() != width as usize * height as usize * 4
        {
            return false;
        }
        self.frame(key, None, (width, height), pixels, sent_ms, 0)
    }
    pub fn jpeg(&self, key: &str, pixels: &[u8], sent_ms: f64) -> bool {
        let Some((width, height)) = jpeg_dimensions(pixels) else {
            return false;
        };
        self.frame(key, None, (width, height), pixels, sent_ms, 1)
    }
    fn frame(
        &self,
        key: &str,
        generation: Option<u64>,
        (width, height): (u32, u32),
        pixels: &[u8],
        sent_ms: f64,
        format: u32,
    ) -> bool {
        if width == 0
            || height == 0
            || width > 3840
            || height > 2160
            || pixels.len() > 3840 * 2160 * 4
        {
            return false;
        }
        let mut surfaces = self.surfaces.lock().unwrap();
        let Some(surface) = surfaces.get_mut(key) else {
            return false;
        };
        if generation.is_some_and(|generation| generation != surface.generation) {
            return false;
        }
        if surface.in_flight.is_some() {
            return false;
        }
        surface.next = surface.next.wrapping_add(1);
        let mut bytes = Vec::with_capacity(24 + pixels.len());
        bytes.extend(surface.next.to_le_bytes());
        bytes.extend(width.to_le_bytes());
        bytes.extend(height.to_le_bytes());
        bytes.extend(format.to_le_bytes());
        bytes.extend(sent_ms.to_le_bytes());
        bytes.extend(pixels);
        surface.in_flight = Some(surface.next);
        if surface.channel.send(Response::new(bytes)).is_err() {
            surfaces.remove(key);
            return false;
        }
        true
    }
    pub fn push(self: &Arc<Self>, session: &str, packet: RemoteVideoPacket) {
        let key = format!("{session}:{}", packet.track.mid);
        let mut surfaces = self.surfaces.lock().unwrap();
        let Some(surface) = surfaces.get_mut(&key) else {
            return;
        };
        if surface.decoder.is_none() {
            tracing::debug!(%key, codec=packet.codec, "starting remote video decoder");
            let (tx, rx) = mpsc::sync_channel(128);
            let hub = Arc::downgrade(self);
            let generation = surface.generation;
            let worker_key = key.clone();
            if let Err(error) = std::thread::Builder::new()
                .name("pingo-video".into())
                .spawn(move || {
                    if let Err(error) = decode(rx, hub, worker_key, generation) {
                        tracing::warn!(%error, "native video decoder stopped");
                    }
                })
            {
                tracing::warn!(%error, "could not start video decoder");
                return;
            }
            surface.decoder = Some(tx);
        }
        if let Some(tx) = &surface.decoder {
            match tx.try_send(packet) {
                Ok(_) | Err(mpsc::TrySendError::Full(_)) => {}
                Err(mpsc::TrySendError::Disconnected(_)) => surface.decoder = None,
            }
        }
    }
}
pub fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.0
}

#[derive(Default)]
struct Assembly {
    timestamp: Option<u32>,
    sequence: Option<u16>,
    data: Vec<u8>,
    valid: bool,
    codec: Option<&'static str>,
    h264: H264Depacketizer,
    vp8: Vp8Depacketizer,
}
impl Assembly {
    fn push(&mut self, p: &RemoteVideoPacket) -> Option<Vec<u8>> {
        let head = if p.codec == "h264" {
            self.h264.is_partition_head(&p.payload)
        } else {
            self.vp8.is_partition_head(&p.payload)
        };
        if self.timestamp != Some(p.timestamp) {
            self.data.clear();
            self.timestamp = Some(p.timestamp);
            self.valid = head;
        }
        if self.codec != Some(p.codec) {
            self.h264 = H264Depacketizer::default();
            self.vp8 = Vp8Depacketizer::default();
            self.codec = Some(p.codec);
        }
        if self
            .sequence
            .is_some_and(|previous| previous.wrapping_add(1) != p.sequence)
        {
            self.valid = false;
        }
        self.sequence = Some(p.sequence);
        if !self.valid {
            return None;
        }
        let result = if p.codec == "h264" {
            self.h264.depacketize(
                &p.payload,
                &mut self.data,
                &mut str0m::format::CodecExtra::None,
            )
        } else {
            self.vp8.depacketize(
                &p.payload,
                &mut self.data,
                &mut str0m::format::CodecExtra::None,
            )
        };
        if result.is_err() || self.data.len() > 8 * 1024 * 1024 {
            self.valid = false;
            self.data.clear();
            return None;
        }
        if p.marker {
            self.valid = false;
            Some(std::mem::take(&mut self.data))
        } else {
            None
        }
    }
}
fn ivf_header(frame: &[u8]) -> Option<Vec<u8>> {
    if frame.len() < 10 || frame[0] & 1 != 0 || frame[3..6] != [0x9d, 0x01, 0x2a] {
        return None;
    }
    let width = u16::from_le_bytes([frame[6], frame[7]]) & 0x3fff;
    let height = u16::from_le_bytes([frame[8], frame[9]]) & 0x3fff;
    if width == 0 || height == 0 || width > 3840 || height > 2160 {
        return None;
    }
    let mut header = b"DKIF\0\0\x20\0VP80".to_vec();
    header.extend(width.to_le_bytes());
    header.extend(height.to_le_bytes());
    header.extend(90000u32.to_le_bytes());
    header.extend(1u32.to_le_bytes());
    header.extend([0u8; 8]);
    Some(header)
}
fn decode(
    rx: mpsc::Receiver<RemoteVideoPacket>,
    hub: Weak<VideoHub>,
    key: String,
    generation: u64,
) -> Result<(), String> {
    let first = rx.recv().map_err(|e| e.to_string())?;
    let codec = first.codec;
    let mut child = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-threads",
            "2",
            "-flags",
            "low_delay",
            "-probesize",
            "32",
            "-analyzeduration",
            "0",
            "-f",
            if codec == "h264" { "h264" } else { "ivf" },
            "-i",
            "pipe:0",
            "-an",
            "-vf",
            "scale=w='min(1920,iw)':h=-2",
            "-threads",
            "2",
            "-q:v",
            "3",
            "-c:v",
            "mjpeg",
            "-f",
            "image2pipe",
            "-flush_packets",
            "1",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Install FFmpeg for native video: {e}"))?;
    let mut input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut output = BufReader::new(output);
        while let Ok(Some(pixels)) = read_jpeg(&mut output) {
            let Some(hub) = hub.upgrade() else { break };
            if let Some(dimensions) = jpeg_dimensions(&pixels) {
                hub.frame(&key, Some(generation), dimensions, &pixels, now_ms(), 1);
            }
        }
    });
    let mut assembly = Assembly::default();
    let mut initialized = false;
    for packet in std::iter::once(first).chain(rx) {
        if packet.codec != codec {
            break;
        }
        if let Some(frame) = assembly.push(&packet) {
            tracing::trace!(codec, bytes = frame.len(), "assembled remote video frame");
            if codec == "vp8" {
                if !initialized {
                    let Some(header) = ivf_header(&frame) else {
                        continue;
                    };
                    if input.write_all(&header).is_err() {
                        break;
                    }
                    initialized = true;
                }
                if input
                    .write_all(&(frame.len() as u32).to_le_bytes())
                    .is_err()
                    || input
                        .write_all(&(packet.timestamp as u64).to_le_bytes())
                        .is_err()
                {
                    break;
                }
            }
            if input.write_all(&frame).is_err() {
                break;
            }
        }
    }
    drop(input);
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    Ok(())
}
pub(crate) fn read_jpeg(reader: &mut impl BufRead) -> std::io::Result<Option<Vec<u8>>> {
    let mut data = Vec::new();
    loop {
        if (&mut *reader)
            .take((8 * 1024 * 1024 + 1 - data.len()) as u64)
            .read_until(0xd9, &mut data)?
            == 0
        {
            return Ok(None);
        }
        if data.len() > 8 * 1024 * 1024 {
            return Err(std::io::Error::other("JPEG frame exceeds limit"));
        }
        if data.ends_with(&[0xff, 0xd9]) {
            break;
        }
    }
    if !data.starts_with(&[0xff, 0xd8]) {
        return Err(std::io::Error::other("Invalid JPEG frame"));
    }
    Ok(Some(data))
}
fn jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if !data.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut i = 2;
    while i + 4 <= data.len() {
        if data[i] != 0xff {
            return None;
        }
        let marker = data[i + 1];
        if marker == 0xff {
            i += 1;
            continue;
        }
        let length = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        if length < 2 || i + 2 + length > data.len() {
            return None;
        }
        if [0xc0, 0xc1, 0xc2].contains(&marker) {
            if length < 8 {
                return None;
            }
            return Some((
                u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32,
                u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32,
            ));
        }
        i += 2 + length;
    }
    None
}
/// PAM carries dimensions with each raw frame, including resolution changes.
pub(crate) fn read_pam(reader: &mut impl BufRead) -> std::io::Result<Option<(u32, u32, Vec<u8>)>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    if line.trim() != "P7" {
        return Err(std::io::Error::other("Invalid PAM magic"));
    }
    let (mut width, mut height, mut depth, mut maxval) = (0, 0, 0, 0);
    for _ in 0..16 {
        line.clear();
        reader.read_line(&mut line)?;
        let mut words = line.split_whitespace();
        match words.next() {
            Some("WIDTH") => width = words.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            Some("HEIGHT") => height = words.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            Some("DEPTH") => depth = words.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            Some("MAXVAL") => maxval = words.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            Some("ENDHDR") => break,
            _ => {}
        }
    }
    if width == 0 || height == 0 || width > 3840 || height > 2160 || depth != 4 || maxval != 255 {
        return Err(std::io::Error::other("Invalid PAM dimensions/format"));
    }
    let mut data = vec![0; width as usize * height as usize * 4];
    reader.read_exact(&mut data)?;
    Ok(Some((width, height, data)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pingo_proto::{TrackInfo, TrackKind};
    use std::time::Duration;
    fn packet(sequence: u16, timestamp: u32, marker: bool, payload: Vec<u8>) -> RemoteVideoPacket {
        RemoteVideoPacket {
            track: TrackInfo {
                participant_id: "p".into(),
                mid: "v".into(),
                kind: TrackKind::Screen,
            },
            codec: "vp8",
            sequence,
            timestamp,
            marker,
            payload,
        }
    }
    #[test]
    fn rtp_loss_drops_incomplete_frames_and_recovers_on_a_new_frame() {
        let mut a = Assembly::default();
        assert!(
            a.push(&packet(1, 100, false, vec![0x10, 0, 1, 2]))
                .is_none()
        );
        assert!(a.push(&packet(3, 100, true, vec![0, 3])).is_none());
        assert!(a.push(&packet(4, 200, true, vec![0x10, 0, 1, 2])).is_some());
    }
    #[test]
    fn truncated_jpeg_dimensions_are_rejected_without_indexing_past_input() {
        // SOF length includes its length word, precision, dimensions and component count.
        assert_eq!(
            jpeg_dimensions(&[0xff, 0xd8, 0xff, 0xc0, 0, 7, 8, 0, 1, 0, 1]),
            None
        );
    }
    #[test]
    fn binary_frames_are_bounded_acknowledged_and_isolated_from_replaced_decoders() {
        let hub = VideoHub::default();
        let (tx, rx) = mpsc::channel();
        let channel = Channel::new(move |body| {
            tx.send(body).unwrap();
            Ok(())
        });
        hub.subscribe("session:mid".into(), channel.clone())
            .unwrap();
        let generation = hub.surfaces.lock().unwrap()["session:mid"].generation;
        assert!(hub.raw("session:mid", 1, 1, &[1, 2, 3, 255], 100.0));
        let tauri::ipc::InvokeResponseBody::Raw(frame) = rx.recv().unwrap() else {
            panic!("binary channel required")
        };
        assert_eq!(frame.len(), 28);
        let id = u32::from_le_bytes(frame[..4].try_into().unwrap());
        assert!(!hub.raw("session:mid", 1, 1, &[0; 4], 100.0));
        hub.ack("session:mid", id.wrapping_add(1));
        assert!(!hub.raw("session:mid", 1, 1, &[0; 4], 100.0));
        hub.ack("session:mid", id);
        assert!(hub.raw("session:mid", 1, 1, &[0; 4], 100.0));
        hub.subscribe("session:mid".into(), channel).unwrap();
        assert!(!hub.frame("session:mid", Some(generation), (1, 1), &[0; 4], 100.0, 0));
        hub.clear_session("session");
        assert!(!hub.raw("session:mid", 1, 1, &[0; 4], 100.0));
    }
    #[test]
    #[ignore = "requires ffmpeg with libx264/libvpx; run explicitly for native video verification"]
    fn ffmpeg_decodes_live_h264_and_vp8_rtp_before_the_stream_closes() {
        use str0m::unversioned::{H264Packetizer, Packetizer, Vp8Packetizer};
        for codec in ["h264", "vp8"] {
            let mut encode = Command::new("ffmpeg");
            encode.args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x64:rate=30",
                "-frames:v",
                "90",
                "-threads",
                "1",
            ]);
            if codec == "h264" {
                encode.args([
                    "-c:v",
                    "libx264",
                    "-preset",
                    "ultrafast",
                    "-tune",
                    "zerolatency",
                    "-x264-params",
                    "aud=1:repeat-headers=1:keyint=1",
                    "-f",
                    "h264",
                ]);
            } else {
                encode.args([
                    "-c:v",
                    "libvpx",
                    "-deadline",
                    "realtime",
                    "-g",
                    "1",
                    "-f",
                    "ivf",
                ]);
            }
            let output = encode.arg("pipe:1").output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let frames: Vec<&[u8]> = if codec == "h264" {
                let mut starts: Vec<_> = output
                    .stdout
                    .windows(5)
                    .enumerate()
                    .filter_map(|(i, b)| (b == [0, 0, 0, 1, 9]).then_some(i))
                    .collect();
                starts.push(output.stdout.len());
                starts
                    .windows(2)
                    .map(|w| &output.stdout[w[0]..w[1]])
                    .collect()
            } else {
                let mut frames = Vec::new();
                let mut at = 32;
                while at + 12 <= output.stdout.len() {
                    let length =
                        u32::from_le_bytes(output.stdout[at..at + 4].try_into().unwrap()) as usize;
                    frames.push(&output.stdout[at + 12..at + 12 + length]);
                    at += 12 + length;
                }
                frames
            };
            assert_eq!(frames.len(), 90);
            let hub = Arc::new(VideoHub::default());
            let (tx, rx) = mpsc::channel();
            hub.subscribe(
                "session:v".into(),
                Channel::new(move |body| {
                    let _ = tx.send(body);
                    Ok(())
                }),
            )
            .unwrap();
            let mut packetizer: Box<dyn Packetizer> = if codec == "h264" {
                Box::new(H264Packetizer::default())
            } else {
                Box::new(Vp8Packetizer::default())
            };
            let mut sequence = 0u16;
            let mut decoded = None;
            for (index, frame) in frames.iter().enumerate() {
                let packets = packetizer.packetize(1200, frame).unwrap();
                for (i, payload) in packets.iter().enumerate() {
                    let mut packet = packet(
                        sequence,
                        index as u32 * 3000,
                        i + 1 == packets.len(),
                        payload.clone(),
                    );
                    packet.codec = codec;
                    hub.push("session", packet);
                    sequence = sequence.wrapping_add(1);
                }
                std::thread::sleep(Duration::from_millis(10));
                if let Ok(body) = rx.try_recv() {
                    decoded = Some(body);
                    break;
                }
            }
            let body = decoded.or_else(|| rx.recv_timeout(Duration::from_secs(5)).ok());
            // Keep the sender open until here: EOF must not be needed to flush a live frame.
            hub.clear_session("session");
            let Some(tauri::ipc::InvokeResponseBody::Raw(bytes)) = body else {
                panic!("{codec}: no decoded binary frame before EOF")
            };
            assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 64);
            assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 64);
            assert_eq!(jpeg_dimensions(&bytes[24..]), Some((64, 64)));
        }
    }
    #[test]
    fn pam_validates_allocation_and_reads_rgba() {
        let mut bytes =
            b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n".to_vec();
        bytes.extend([1, 2, 3, 255]);
        assert_eq!(
            read_pam(&mut &bytes[..]).unwrap().unwrap(),
            (1, 1, vec![1, 2, 3, 255])
        );
        assert!(
            read_pam(&mut &b"P7\nWIDTH 99999\nHEIGHT 99999\nDEPTH 4\nMAXVAL 255\nENDHDR\n"[..])
                .is_err()
        );
    }
}
