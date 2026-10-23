//! A bounded, timestamp-driven playout buffer. Unlike str0m's depacketizer this
//! deliberately owns loss recovery and the audio clock, so Opus FEC remains usable.
use super::{
    codec::{Decoder, EncodedPacket, PacketDuration, Profile},
    level::rms,
};
use crate::{Error, FRAME_SAMPLES, Result};
use std::collections::{BTreeMap, VecDeque};

const WINDOW: usize = 200;
const MAX_PACKETS: usize = 64;
const MAX_DELAY_MS: f64 = 200.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct JitterStats {
    pub current_delay_ms: f32,
    pub target_delay_ms: f32,
    pub jitter_ms: f32,
    pub loss_percent: f32,
    pub concealed_percent: f32,
    pub late_percent: f32,
    pub received: u64,
    pub duplicates: u64,
    pub late: u64,
    pub played_packets: u64,
    pub lost_packets: u64,
    /// libopus can return PLC when LBRR is absent; this counts attempts, not proven recoveries.
    pub fec_attempts: u64,
    pub plc_packets: u64,
    pub corrupt_packets: u64,
    pub expanded_samples: u64,
    pub compressed_samples: u64,
    pub resets: u64,
}

struct Packet {
    packet: EncodedPacket,
    timestamp: i64,
}

pub struct JitterBuffer {
    decoder: Decoder,
    packets: BTreeMap<i64, Packet>,
    seen: VecDeque<i64>,
    high_sequence: Option<i64>,
    high_timestamp: i64,
    next_sequence: Option<i64>,
    next_timestamp: i64,
    play_cursor: f64,
    started: bool,
    first_arrival_ms: f64,
    transits: VecDeque<f64>,
    baseline: f64,
    duration: PacketDuration,
    pcm: VecDeque<f32>,
    last_rms: f32,
    stats: JitterStats,
}

impl JitterBuffer {
    pub fn new(profile: Profile) -> Result<Self> {
        Ok(Self {
            decoder: Decoder::new(profile)?,
            packets: BTreeMap::new(),
            seen: VecDeque::with_capacity(512),
            high_sequence: None,
            high_timestamp: 0,
            next_sequence: None,
            next_timestamp: 0,
            play_cursor: 0.0,
            started: false,
            first_arrival_ms: 0.0,
            transits: VecDeque::with_capacity(WINDOW),
            baseline: 0.0,
            duration: PacketDuration::Default20,
            pcm: VecDeque::new(),
            last_rms: 0.0,
            stats: JitterStats {
                target_delay_ms: 20.0,
                ..Default::default()
            },
        })
    }

    pub fn channels(&self) -> usize {
        self.decoder.channels
    }
    pub fn buffered_packets(&self) -> usize {
        self.packets.len()
    }
    pub fn stats(&self) -> JitterStats {
        let mut stats = self.stats;
        let played = stats.played_packets.max(1) as f32;
        stats.loss_percent = 100.0 * stats.lost_packets as f32 / played;
        stats.concealed_percent = stats.loss_percent;
        stats.late_percent = 100.0 * stats.late as f32 / stats.received.max(1) as f32;
        stats
    }

    /// Arrival time is monotonic milliseconds, relative to any fixed local epoch.
    /// Sequence/timestamp wrap are extended independently before ordering.
    pub fn push(&mut self, packet: EncodedPacket, arrival_ms: f64) -> Result<bool> {
        if !arrival_ms.is_finite() || packet.payload.is_empty() || packet.payload.len() > 4000 {
            return Err(Error::Invalid("invalid RTP packet or arrival time"));
        }
        let samples = opus::packet::get_nb_samples(&packet.payload, 48_000)?;
        if samples != packet.duration.samples() {
            return Err(Error::Invalid("RTP duration disagrees with Opus packet"));
        }
        let sequence = match self.high_sequence {
            Some(high) => high + packet.sequence.wrapping_sub(high as u16) as i16 as i64,
            None => packet.sequence as i64,
        };
        let timestamp = if self.high_sequence.is_some() {
            self.high_timestamp
                + packet.timestamp.wrapping_sub(self.high_timestamp as u32) as i32 as i64
        } else {
            packet.timestamp as i64
        };
        self.stats.received += 1;
        if self.seen.contains(&sequence) {
            self.stats.duplicates += 1;
            return Ok(false);
        }
        // Do not let a corrupt header allocate an unbounded queue or force hours of PLC.
        if let Some(high) = self.high_sequence
            && ((sequence - high).abs() > 512
                || (timestamp - self.high_timestamp).abs() > 48_000 * 10)
        {
            return Err(Error::Invalid(
                "RTP discontinuity: recreate the participant buffer",
            ));
        }
        self.seen.push_back(sequence);
        if self.seen.len() > 512 {
            self.seen.pop_front();
        }
        if self.high_sequence.is_none_or(|high| sequence > high) {
            self.high_sequence = Some(sequence);
            self.high_timestamp = timestamp;
        }
        if !self.started && self.next_sequence.is_some() {
            self.first_arrival_ms = self.first_arrival_ms.min(arrival_ms);
        }
        let transit = arrival_ms - timestamp as f64 / 48.0;
        self.transits.push_back(transit);
        if self.transits.len() > WINDOW {
            self.transits.pop_front();
        }
        let mut sorted: Vec<_> = self.transits.iter().copied().collect();
        sorted.sort_unstable_by(f64::total_cmp);
        self.baseline = sorted[0];
        let spread = sorted[((sorted.len() - 1) * 95).div_ceil(100)] - self.baseline;
        self.stats.jitter_ms = spread as f32;
        let desired = (20.0 + spread).clamp(20.0, MAX_DELAY_MS) as f32;
        // Grow quickly when the network changes; decay slowly to avoid oscillation.
        self.stats.target_delay_ms = if desired > self.stats.target_delay_ms {
            desired
        } else {
            self.stats.target_delay_ms * 0.995 + desired * 0.005
        };
        // Late packets are still jitter observations. Ignoring them creates a
        // feedback loop in which bad playout keeps the estimated delay too low.
        if self.started && sequence < self.next_sequence.unwrap_or(sequence) {
            self.stats.late += 1;
            return Ok(false);
        }
        if self.next_sequence.is_none() {
            self.first_arrival_ms = arrival_ms;
            self.next_sequence = Some(sequence);
            self.next_timestamp = timestamp;
            self.play_cursor = timestamp as f64;
            self.duration = packet.duration;
        } else if !self.started && sequence < self.next_sequence.unwrap() {
            self.next_sequence = Some(sequence);
            self.next_timestamp = timestamp;
            self.play_cursor = timestamp as f64;
        }
        self.packets.insert(sequence, Packet { packet, timestamp });
        while self.packets.len() > MAX_PACKETS {
            self.packets.pop_first();
        }
        Ok(true)
    }

    fn decode_next(&mut self) -> Result<()> {
        let sequence = self.next_sequence.unwrap();
        let pcm = if let Some(packet) = self.packets.remove(&sequence) {
            // Accommodate live packet-duration changes and timestamp-based DTX gaps.
            if packet.timestamp > self.next_timestamp {
                let gap = packet.timestamp - self.next_timestamp;
                if gap <= 9600 {
                    self.pcm
                        .extend(std::iter::repeat_n(0.0, gap as usize * self.channels()));
                }
            }
            self.duration = packet.packet.duration;
            self.next_timestamp = packet.timestamp;
            match self
                .decoder
                .decode(Some(&packet.packet.payload), self.duration.samples(), false)
            {
                Ok(pcm) => pcm,
                Err(_) => {
                    self.stats.corrupt_packets += 1;
                    self.stats.lost_packets += 1;
                    self.stats.plc_packets += 1;
                    self.decoder.decode(None, self.duration.samples(), false)?
                }
            }
        } else {
            self.stats.lost_packets += 1;
            if let Some(next) = self.packets.get(&(sequence + 1)) {
                // RTP timestamps disambiguate a packet-duration change at a loss.
                self.duration = match next.timestamp - self.next_timestamp {
                    480 => PacketDuration::LowLatency10,
                    960 => PacketDuration::Default20,
                    _ => self.duration,
                };
                self.stats.fec_attempts += 1;
                match self
                    .decoder
                    .decode(Some(&next.packet.payload), self.duration.samples(), true)
                {
                    Ok(pcm) => pcm,
                    Err(_) => {
                        self.stats.plc_packets += 1;
                        self.decoder.decode(None, self.duration.samples(), false)?
                    }
                }
            } else {
                self.stats.plc_packets += 1;
                self.decoder.decode(None, self.duration.samples(), false)?
            }
        };
        self.stats.played_packets += 1;
        self.pcm.extend(pcm);
        self.next_sequence = Some(sequence + 1);
        self.next_timestamp += self.duration.samples() as i64;
        self.packets.retain(|s, _| *s > sequence);
        Ok(())
    }

    /// Always emits exactly 10 ms. Call once per local audio tick, including during loss.
    pub fn render(&mut self, now_ms: f64) -> Result<Vec<f32>> {
        if !now_ms.is_finite() {
            return Err(Error::Invalid("invalid playout time"));
        }
        let channels = self.channels();
        if self.next_sequence.is_none()
            || (!self.started && now_ms < self.first_arrival_ms + self.stats.target_delay_ms as f64)
        {
            return Ok(vec![0.0; FRAME_SAMPLES * channels]);
        }
        self.started = true;
        let mut delay = (now_ms - self.baseline - self.play_cursor / 48.0).max(0.0);
        // A suspended worker cannot catch up gradually. Re-anchor within the hard bound.
        if delay > MAX_DELAY_MS + 10.0 {
            if let Some((&sequence, packet)) = self
                .packets
                .iter()
                .find(|(_, p)| now_ms - self.baseline - p.timestamp as f64 / 48.0 <= MAX_DELAY_MS)
            {
                self.next_sequence = Some(sequence);
                self.next_timestamp = packet.timestamp;
                self.play_cursor = packet.timestamp as f64;
                self.packets.retain(|s, _| *s >= sequence);
            } else {
                // No fresh packet exists after suspension. Skip expired slots
                // instead of reporting seconds of delay or decoding stale audio.
                let skip = ((delay - MAX_DELAY_MS) / self.duration.millis() as f64).ceil() as i64;
                let skipped_samples = skip * self.duration.samples() as i64;
                self.next_sequence = self.next_sequence.map(|sequence| sequence + skip);
                self.next_timestamp += skipped_samples;
                self.play_cursor += skipped_samples as f64;
                self.stats.lost_packets += skip as u64;
                self.stats.plc_packets += skip as u64;
                self.stats.played_packets += skip as u64;
                self.packets.clear();
            }
            self.pcm.clear();
            self.stats.resets += 1;
            delay = (now_ms - self.baseline - self.play_cursor / 48.0).max(0.0);
        }
        self.stats.current_delay_ms = delay as f32;
        let error = delay - self.stats.target_delay_ms as f64;
        // Speech normally gets 1% adjustment (5% on a large delay step);
        // quiet material tolerates 10% to converge faster.
        let adjustment = if self.last_rms < 0.015 {
            48
        } else if error.abs() > 30.0 {
            24
        } else {
            5
        };
        let consume = if error > 3.0 {
            FRAME_SAMPLES + adjustment
        } else if error < -3.0 {
            FRAME_SAMPLES - adjustment
        } else {
            FRAME_SAMPLES
        };
        while self.pcm.len() < consume * channels {
            self.decode_next()?;
        }
        let input: Vec<_> = self.pcm.drain(..consume * channels).collect();
        let output = overlap_add(&input, channels);
        self.last_rms = rms(&output);
        self.play_cursor += consume as f64;
        if consume > FRAME_SAMPLES {
            self.stats.compressed_samples += (consume - FRAME_SAMPLES) as u64;
        } else {
            self.stats.expanded_samples += (FRAME_SAMPLES - consume) as u64;
        }
        Ok(output)
    }
}

/// Splice two overlapping sections with a raised-cosine crossfade. All channels
/// share the splice so stereo phase is preserved. This is intentionally simpler
/// than pitch-search WSOLA; the controller prefers quiet frames for larger moves.
fn overlap_add(input: &[f32], channels: usize) -> Vec<f32> {
    let frames = input.len() / channels;
    if frames == FRAME_SAMPLES {
        return input.to_vec();
    }
    let shift = frames as isize - FRAME_SAMPLES as isize;
    let start = 144;
    let width = 144;
    let mut output = vec![0.0; FRAME_SAMPLES * channels];
    for i in 0..FRAME_SAMPLES {
        let mix = if i < start {
            0.0
        } else if i >= start + width {
            1.0
        } else {
            0.5 - 0.5 * (std::f32::consts::PI * (i - start) as f32 / width as f32).cos()
        };
        let a = i.min(frames - 1);
        let b = (i as isize + shift).clamp(0, frames as isize - 1) as usize;
        for ch in 0..channels {
            output[i * channels + ch] =
                input[a * channels + ch] * (1.0 - mix) + input[b * channels + ch] * mix;
        }
    }
    output
}
