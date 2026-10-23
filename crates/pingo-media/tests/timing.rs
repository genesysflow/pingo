//! Run with: cargo test -p pingo-media --release --test timing -- --ignored --nocapture
use pingo_media::{
    FRAME_SAMPLES,
    pipeline::{AudioSettings, ReceivePipeline, SendPipeline},
};
use std::{hint::black_box, time::Instant};

fn signal(sample: usize) -> f32 {
    let t = sample as f32 / 48_000.0;
    let phase = std::f32::consts::TAU * (170.0 * t + 400.0 * t * t);
    (phase.sin() + 0.35 * (phase * 2.13).sin()) * (0.10 + 0.04 * (t * 37.0).sin())
}

#[test]
#[ignore = "release CPU/latency measurement; no audio hardware or network"]
fn pipeline_cpu_and_latency() {
    let mut send = SendPipeline::new(AudioSettings::default()).unwrap();
    let mut receive = ReceivePipeline::new(AudioSettings::default());
    receive.add_participant("test".into()).unwrap();
    let mut timings = Vec::new();
    for i in 0..5200 {
        let input = std::array::from_fn(|j| signal(i * FRAME_SAMPLES + j));
        let start = Instant::now();
        if let Some(packet) = send.process_mono(black_box(&input)).unwrap() {
            receive.push("test", packet, (i + 1) as f64 * 10.0).unwrap();
        }
        let output = receive.render((i + 1) as f64 * 10.0).unwrap();
        send.render_reference(&output).unwrap();
        black_box(output);
        if i >= 200 {
            timings.push(start.elapsed().as_secs_f64() * 1e6);
        }
    }
    timings.sort_by(f64::total_cmp);
    let mean = timings.iter().sum::<f64>() / timings.len() as f64;
    println!(
        "default mono send + one stereo receive/mix: mean {mean:.2} us/10ms, median {:.2} us, p95 {:.2} us, CPU {:.2}% of one core",
        timings[timings.len() / 2],
        timings[timings.len() * 95 / 100],
        mean / 100.0
    );
    println!(
        "default conservative algorithmic budget (20 ms playout, capture+playback quanta): {:.2} ms",
        send.latency_budget_ms(20.0).unwrap()
    );

    let mut send = SendPipeline::new(AudioSettings::default()).unwrap();
    let mut receive = ReceivePipeline::new(AudioSettings::default());
    receive.add_participant("test".into()).unwrap();
    let mut input = Vec::new();
    let mut output = Vec::new();
    for i in 0..150 {
        let frame = std::array::from_fn(|j| signal(i * FRAME_SAMPLES + j));
        input.extend(frame);
        if let Some(packet) = send.process_mono(&frame).unwrap() {
            receive.push("test", packet, (i + 1) as f64 * 10.0).unwrap();
        }
        let rendered = receive.render((i + 1) as f64 * 10.0).unwrap();
        output.extend(rendered.map(|s| s[0]));
    }
    // Correlate a chirp after startup, at 12 kHz analysis rate (0.083 ms resolution).
    let mut best = (f64::NEG_INFINITY, 0);
    for lag in (0..7200).step_by(4) {
        let mut cross = 0.0f64;
        let mut a = 0.0;
        let mut b = 0.0;
        for i in (9600..48000).step_by(4) {
            let x = input[i] as f64;
            let y = output[i + lag] as f64;
            cross += x * y;
            a += x * x;
            b += y * y;
        }
        let correlation = cross / (a * b).sqrt().max(1e-20);
        if correlation > best.0 {
            best = (correlation, lag);
        }
    }
    let timeline_ms = best.1 as f64 / 48.0;
    println!(
        "sample timeline delay: {timeline_ms:.2} ms, correlation {:.3}; capture-completion clock +10 ms, conservative playback quantum +10 ms => {:.2} ms",
        best.0,
        timeline_ms + 20.0
    );
    assert!(best.0 > 0.4, "pipeline must preserve correlated signal");
    assert!(timeline_ms + 20.0 < 150.0);
}
