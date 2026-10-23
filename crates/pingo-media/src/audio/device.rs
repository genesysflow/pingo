//! Only sample conversion, bounded SPSC queue operations and atomic counters run
//! in CPAL callbacks. Enumeration, resampling and recovery belong to the worker.
use crate::{Error, FRAME_SAMPLES, Result, SAMPLE_RATE, StereoFrame};
use cpal::{
    FromSample, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub name: String,
    pub input: bool,
    pub is_default: bool,
}

pub fn list_devices() -> Result<Vec<DeviceInfo>> {
    let host = cpal::default_host();
    let mut result = Vec::new();
    for input in [true, false] {
        let default = if input {
            host.default_input_device()
        } else {
            host.default_output_device()
        };
        let default_name = default.and_then(|d| d.name().ok());
        let devices = if input {
            host.input_devices()
        } else {
            host.output_devices()
        }
        .map_err(|e| Error::Device(e.to_string()))?;
        for device in devices {
            if let Ok(name) = device.name() {
                result.push(DeviceInfo {
                    is_default: default_name.as_ref() == Some(&name),
                    name,
                    input,
                });
            }
        }
    }
    Ok(result)
}

#[derive(Default)]
struct Counters {
    input_callback_frames: AtomicU64,
    input_latency_us: AtomicU64,
    output_latency_us: AtomicU64,
    fault: AtomicBool,
    input_overruns: AtomicU64,
    output_underruns: AtomicU64,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct DeviceStats {
    pub input_overruns: u64,
    pub output_underruns: u64,
    pub fault: bool,
}

/// Streaming sinc resampling outside the callbacks. Interleaved stereo is also
/// used for mono devices, keeping ring writes atomic at a sample-frame boundary.
pub struct RateConverter {
    resampler: Option<SincFixedIn<f32>>,
    pending: VecDeque<[f32; 2]>,
    chunk: usize,
}
impl RateConverter {
    pub fn new(input_rate: u32, output_rate: u32, chunk: usize) -> Result<Self> {
        if input_rate == 0 || output_rate == 0 || chunk == 0 {
            return Err(Error::Invalid("invalid resampler rate/chunk"));
        }
        let resampler = if input_rate == output_rate {
            None
        } else {
            Some(
                SincFixedIn::new(
                    output_rate as f64 / input_rate as f64,
                    1.01,
                    SincInterpolationParameters {
                        sinc_len: 128,
                        f_cutoff: 0.92,
                        interpolation: SincInterpolationType::Cubic,
                        oversampling_factor: 128,
                        window: WindowFunction::BlackmanHarris2,
                    },
                    chunk,
                    2,
                )
                .map_err(|e| Error::Processing(e.to_string()))?,
            )
        };
        Ok(Self {
            resampler,
            pending: VecDeque::new(),
            chunk,
        })
    }
    pub fn delay_samples(&self) -> usize {
        self.resampler.as_ref().map_or(0, Resampler::output_delay)
    }
    pub fn process(&mut self, samples: &[[f32; 2]]) -> Result<Vec<[f32; 2]>> {
        if self.resampler.is_none() {
            return Ok(samples.to_vec());
        }
        self.pending.extend(samples);
        let mut output = Vec::new();
        while self.pending.len() >= self.chunk {
            let mut input = [
                Vec::with_capacity(self.chunk),
                Vec::with_capacity(self.chunk),
            ];
            for sample in self.pending.drain(..self.chunk) {
                input[0].push(sample[0]);
                input[1].push(sample[1]);
            }
            let pcm = self
                .resampler
                .as_mut()
                .unwrap()
                .process(&input, None)
                .map_err(|e| Error::Processing(e.to_string()))?;
            output.extend(pcm[0].iter().zip(&pcm[1]).map(|(&l, &r)| [l, r]));
        }
        Ok(output)
    }
}

pub struct DeviceIo {
    _input_stream: cpal::Stream,
    _output_stream: cpal::Stream,
    capture: rtrb::Consumer<[f32; 2]>,
    playback: rtrb::Producer<[f32; 2]>,
    input_resampler: RateConverter,
    output_resampler: RateConverter,
    captured: VecDeque<[f32; 2]>,
    counters: Arc<Counters>,
    input_name: Option<String>,
    output_name: Option<String>,
    input_chunk: usize,
    next_retry: Instant,
    pub input_rate: u32,
    pub output_rate: u32,
}

fn select(name: Option<&str>, input: bool) -> Result<cpal::Device> {
    let host = cpal::default_host();
    if let Some(name) = name {
        let mut devices = if input {
            host.input_devices()
        } else {
            host.output_devices()
        }
        .map_err(|e| Error::Device(e.to_string()))?;
        if let Some(device) = devices.find(|d| d.name().is_ok_and(|n| n == name)) {
            return Ok(device);
        }
        tracing::warn!(
            device = name,
            "selected audio device disappeared; using default"
        );
    }
    (if input {
        host.default_input_device()
    } else {
        host.default_output_device()
    })
    .ok_or_else(|| {
        Error::Device(format!(
            "no default {} device",
            if input { "input" } else { "output" }
        ))
    })
}

fn configuration(device: &cpal::Device, input: bool) -> Result<cpal::SupportedStreamConfig> {
    let configs: Vec<_> = if input {
        device.supported_input_configs().map(|c| c.collect())
    } else {
        device.supported_output_configs().map(|c| c.collect())
    }
    .map_err(|e| Error::Device(e.to_string()))?;
    if let Some(config) = configs
        .into_iter()
        .filter(|c| {
            c.channels() <= 2
                && c.min_sample_rate().0 <= SAMPLE_RATE
                && c.max_sample_rate().0 >= SAMPLE_RATE
        })
        .max_by_key(|c| (c.channels(), c.sample_format() == cpal::SampleFormat::F32))
    {
        return Ok(config.with_sample_rate(cpal::SampleRate(SAMPLE_RATE)));
    }
    (if input {
        device.default_input_config()
    } else {
        device.default_output_config()
    })
    .map_err(|e| Error::Device(e.to_string()))
}

fn input_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut queue: rtrb::Producer<[f32; 2]>,
    counters: Arc<Counters>,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    let errors = counters.clone();
    device
        .build_input_stream(
            config,
            move |data: &[T], info| {
                counters
                    .input_callback_frames
                    .store((data.len() / channels) as u64, Ordering::Relaxed);
                let stamp = info.timestamp();
                if let Some(age) = stamp.callback.duration_since(&stamp.capture) {
                    counters
                        .input_latency_us
                        .store(age.as_micros() as u64, Ordering::Relaxed);
                }
                for frame in data.chunks_exact(channels) {
                    let sample = [
                        frame[0].to_sample::<f32>(),
                        frame[channels.min(2) - 1].to_sample::<f32>(),
                    ];
                    if queue.push(sample).is_err() {
                        counters.input_overruns.fetch_add(1, Ordering::Relaxed);
                    }
                }
            },
            move |_| {
                errors.fault.store(true, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|e| Error::Device(e.to_string()))
}
fn output_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut queue: rtrb::Consumer<[f32; 2]>,
    counters: Arc<Counters>,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let errors = counters.clone();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], info| {
                let stamp = info.timestamp();
                if let Some(delay) = stamp.playback.duration_since(&stamp.callback) {
                    counters
                        .output_latency_us
                        .store(delay.as_micros() as u64, Ordering::Relaxed);
                }
                for frame in data.chunks_exact_mut(channels) {
                    let sample = queue.pop().unwrap_or_else(|_| {
                        counters.output_underruns.fetch_add(1, Ordering::Relaxed);
                        [0.0; 2]
                    });
                    for (ch, dst) in frame.iter_mut().enumerate() {
                        let value = if channels == 1 {
                            (sample[0] + sample[1]) * 0.5
                        } else if ch < 2 {
                            sample[ch]
                        } else {
                            0.0
                        };
                        *dst = T::from_sample(value);
                    }
                }
            },
            move |_| {
                errors.fault.store(true, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|e| Error::Device(e.to_string()))
}

macro_rules! stream {
    ($function:ident, $device:expr, $supported:expr, $queue:expr, $counters:expr) => {
        match $supported.sample_format() {
            cpal::SampleFormat::F32 => {
                $function::<f32>($device, &$supported.config(), $queue, $counters)
            }
            cpal::SampleFormat::F64 => {
                $function::<f64>($device, &$supported.config(), $queue, $counters)
            }
            cpal::SampleFormat::I8 => {
                $function::<i8>($device, &$supported.config(), $queue, $counters)
            }
            cpal::SampleFormat::I16 => {
                $function::<i16>($device, &$supported.config(), $queue, $counters)
            }
            cpal::SampleFormat::I32 => {
                $function::<i32>($device, &$supported.config(), $queue, $counters)
            }
            cpal::SampleFormat::I64 => {
                $function::<i64>($device, &$supported.config(), $queue, $counters)
            }
            cpal::SampleFormat::U8 => {
                $function::<u8>($device, &$supported.config(), $queue, $counters)
            }
            cpal::SampleFormat::U16 => {
                $function::<u16>($device, &$supported.config(), $queue, $counters)
            }
            cpal::SampleFormat::U32 => {
                $function::<u32>($device, &$supported.config(), $queue, $counters)
            }
            cpal::SampleFormat::U64 => {
                $function::<u64>($device, &$supported.config(), $queue, $counters)
            }
            _ => Err(Error::Device("unsupported device sample format".into())),
        }
    };
}

impl DeviceIo {
    pub fn open(input_name: Option<String>, output_name: Option<String>) -> Result<Self> {
        let input = select(input_name.as_deref(), true)?;
        let output = select(output_name.as_deref(), false)?;
        let input_config = configuration(&input, true)?;
        let output_config = configuration(&output, false)?;
        let input_rate = input_config.sample_rate().0;
        let output_rate = output_config.sample_rate().0;
        let input_chunk = (input_rate / 100) as usize;
        let (input_queue, capture) = rtrb::RingBuffer::new(input_rate as usize / 10);
        let (playback, output_queue) = rtrb::RingBuffer::new(output_rate as usize / 10);
        let counters = Arc::new(Counters::default());
        let input_stream = stream!(
            input_stream,
            &input,
            input_config,
            input_queue,
            counters.clone()
        )?;
        let output_stream = stream!(
            output_stream,
            &output,
            output_config,
            output_queue,
            counters.clone()
        )?;
        input_stream
            .play()
            .map_err(|e| Error::Device(e.to_string()))?;
        output_stream
            .play()
            .map_err(|e| Error::Device(e.to_string()))?;
        Ok(Self {
            _input_stream: input_stream,
            _output_stream: output_stream,
            capture,
            playback,
            input_resampler: RateConverter::new(input_rate, SAMPLE_RATE, input_chunk)?,
            output_resampler: RateConverter::new(SAMPLE_RATE, output_rate, FRAME_SAMPLES)?,
            captured: VecDeque::new(),
            counters,
            input_name,
            output_name,
            input_chunk,
            next_retry: Instant::now(),
            input_rate,
            output_rate,
        })
    }

    /// CPAL hardware timestamps plus the worker's queued/resampler samples.
    /// Read before capture() and playback() to include the current capture backlog.
    pub fn stream_delay_ms(&self) -> u16 {
        let hardware_us = self.counters.input_latency_us.load(Ordering::Relaxed)
            + self.counters.output_latency_us.load(Ordering::Relaxed);
        // capture timestamp refers to the first sample of the last callback;
        // its buffer duration is already included in input_latency_us.
        let queued_capture = self
            .capture
            .slots()
            .saturating_sub(self.counters.input_callback_frames.load(Ordering::Relaxed) as usize);
        let capture_ms = queued_capture as f64 * 1000.0 / self.input_rate as f64
            + (self.captured.len() + self.input_resampler.delay_samples()) as f64 / 48.0;
        let playback_ms = (self.playback.buffer().capacity() - self.playback.slots()) as f64
            * 1000.0
            / self.output_rate as f64
            + self.output_resampler.delay_samples() as f64 * 1000.0 / self.output_rate as f64;
        (hardware_us as f64 / 1000.0 + capture_ms + playback_ms)
            .round()
            .clamp(0.0, 500.0) as u16
    }

    pub fn stats(&self) -> DeviceStats {
        DeviceStats {
            input_overruns: self.counters.input_overruns.load(Ordering::Relaxed),
            output_underruns: self.counters.output_underruns.load(Ordering::Relaxed),
            fault: self.counters.fault.load(Ordering::Relaxed),
        }
    }
    /// Call on the worker after a device error; failed reopen leaves retry possible.
    pub fn recover(&mut self) -> Result<bool> {
        if !self.stats().fault || Instant::now() < self.next_retry {
            return Ok(false);
        }
        self.next_retry = Instant::now() + Duration::from_secs(1);
        *self = Self::open(self.input_name.clone(), self.output_name.clone())?;
        Ok(true)
    }
    /// Device switches are transactional: the old streams survive a failed open.
    pub fn select_devices(&mut self, input: Option<String>, output: Option<String>) -> Result<()> {
        *self = Self::open(input, output)?;
        Ok(())
    }
    pub fn capture(&mut self) -> Result<Option<StereoFrame>> {
        while self.captured.len() < FRAME_SAMPLES && self.capture.slots() >= self.input_chunk {
            let input: Vec<_> = (0..self.input_chunk)
                .filter_map(|_| self.capture.pop().ok())
                .collect();
            self.captured.extend(self.input_resampler.process(&input)?);
        }
        if self.captured.len() < FRAME_SAMPLES {
            return Ok(None);
        }
        let mut frame = [[0.0; 2]; FRAME_SAMPLES];
        for sample in &mut frame {
            *sample = self.captured.pop_front().unwrap();
        }
        Ok(Some(frame))
    }
    /// Returns false on a full ring, so a stalled producer never blocks the callback.
    pub fn playback(&mut self, frame: &StereoFrame) -> Result<bool> {
        let pcm = self.output_resampler.process(frame)?;
        if self.playback.slots() < pcm.len() {
            return Ok(false);
        }
        for sample in pcm {
            let _ = self.playback.push(sample);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires audio hardware"]
    fn open_default_streams() {
        super::DeviceIo::open(None, None).unwrap();
    }
}
