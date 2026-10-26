use pingo_denoise::{DeepFilter, HOP, Strength};
use std::f32::consts::TAU;

struct Noise(u64);
impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 as u32 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// Voiced "speech": a glottal-like harmonic source with drifting pitch through
/// two formant resonators, gated into syllables with short pauses.
fn speech(seconds: f32) -> Vec<f32> {
    let n = (seconds * 48_000.0) as usize;
    let mut phase = 0.0f32;
    let mut out = Vec::with_capacity(n);
    let mut resonators = [(0.0f32, 0.0f32); 2];
    for i in 0..n {
        let t = i as f32 / 48_000.0;
        let pitch = 120.0 + 25.0 * (TAU * 0.7 * t).sin() + 8.0 * (TAU * 5.0 * t).sin();
        phase = (phase + pitch / 48_000.0).fract();
        let source = if phase < 0.4 {
            (TAU * phase / 0.8).sin()
        } else {
            0.0
        } - 0.25;
        let syllable = (t * 3.2).fract();
        let envelope = if syllable < 0.75 {
            (std::f32::consts::PI * syllable / 0.75).sin()
        } else {
            0.0
        };
        let vowel = (t * 1.6).floor() as usize % 3;
        let formants = [
            [(700.0, 90.0), (1220.0, 110.0)],
            [(300.0, 60.0), (2300.0, 150.0)],
            [(500.0, 80.0), (900.0, 100.0)],
        ][vowel];
        let mut y = 0.0;
        for (state, (f, bw)) in resonators.iter_mut().zip(formants) {
            let r = (-std::f32::consts::PI * bw / 48_000.0).exp();
            let a1 = 2.0 * r * (TAU * f / 48_000.0).cos();
            let a2 = -r * r;
            let v = source * (1.0 - r) + a1 * state.0 + a2 * state.1;
            state.1 = state.0;
            state.0 = v;
            y += v;
        }
        out.push(0.25 * envelope * y);
    }
    out
}

fn energy(x: &[f32]) -> f64 {
    x.iter().map(|&v| (v as f64).powi(2)).sum()
}

fn si_sdr(estimate: &[f32], reference: &[f32]) -> f64 {
    let dot: f64 = estimate
        .iter()
        .zip(reference)
        .map(|(&a, &b)| a as f64 * b as f64)
        .sum();
    let alpha = dot / energy(reference);
    let (mut target, mut error) = (0.0, 0.0);
    for (&e, &r) in estimate.iter().zip(reference) {
        let t = alpha * r as f64;
        target += t * t;
        error += (e as f64 - t).powi(2);
    }
    10.0 * (target / error).log10()
}

fn run(filter: &mut DeepFilter, input: &[f32]) -> Vec<f32> {
    input
        .as_chunks::<HOP>()
        .0
        .iter()
        .flat_map(|chunk| {
            let mut frame = *chunk;
            let lsnr = filter.process(&mut frame).unwrap();
            assert!(lsnr.is_finite());
            assert!(frame.iter().all(|v| v.is_finite()));
            frame
        })
        .collect()
}

fn white(seconds: f32, amplitude: f32, seed: u64) -> Vec<f32> {
    let mut rng = Noise(seed);
    (0..(seconds * 48_000.0) as usize)
        .map(|_| rng.next() * amplitude)
        .collect()
}

/// Low-passed rumble plus mains hum, like a laptop fan next to the mic.
fn fan(seconds: f32) -> Vec<f32> {
    let mut rng = Noise(11);
    let mut lowpass = 0.0f32;
    (0..(seconds * 48_000.0) as usize)
        .map(|i| {
            lowpass += 0.2 * (rng.next() - lowpass);
            0.3 * lowpass + 0.02 * (TAU * 100.0 * i as f32 / 48_000.0).sin()
        })
        .collect()
}

fn reduction_db(input: &[f32], output: &[f32]) -> f64 {
    // Skip the first second while the model's running normalization settles.
    10.0 * (energy(&input[48_000..]) / energy(&output[48_000..])).log10()
}

#[test]
fn speech_passes_through_with_the_reported_delay() {
    let clean = speech(4.0);
    for strength in [Strength::Gentle, Strength::Strong] {
        let mut filter = DeepFilter::new(strength).unwrap();
        let delay = filter.delay_samples();
        assert_eq!(
            delay, 1440,
            "30 ms: STFT overlap plus two frames of lookahead"
        );
        let out = run(&mut filter, &clean);
        let sdr = si_sdr(&out[delay..], &clean[..clean.len() - delay]);
        let gain = 10.0 * (energy(&out) / energy(&clean)).log10();
        assert!(
            sdr > 20.0,
            "{strength:?} distorted clean speech: SI-SDR {sdr:.1} dB"
        );
        assert!(
            gain.abs() < 1.5,
            "{strength:?} changed speech level by {gain:.1} dB"
        );
    }
}

#[test]
fn gentle_caps_attenuation_and_strong_removes_noise() {
    let noise = white(4.0, 0.05, 7);
    let gentle = reduction_db(
        &noise,
        &run(&mut DeepFilter::new(Strength::Gentle).unwrap(), &noise),
    );
    let strong = reduction_db(
        &noise,
        &run(&mut DeepFilter::new(Strength::Strong).unwrap(), &noise),
    );
    assert!(
        (12.0..=19.0).contains(&gentle),
        "gentle reduced noise by {gentle:.1} dB"
    );
    assert!(strong > 35.0, "strong reduced noise by {strong:.1} dB");
}

#[test]
fn noisy_speech_gets_cleaner() {
    let clean = speech(5.0);
    let noise = fan(5.0);
    let mix: Vec<f32> = clean.iter().zip(&noise).map(|(s, n)| s + n).collect();
    let before = si_sdr(&mix, &clean);
    for strength in [Strength::Gentle, Strength::Strong] {
        let mut filter = DeepFilter::new(strength).unwrap();
        let delay = filter.delay_samples();
        let out = run(&mut filter, &mix);
        let after = si_sdr(&out[delay..], &clean[..clean.len() - delay]);
        assert!(
            after > before + 6.0,
            "{strength:?}: SI-SDR {before:.1} -> {after:.1} dB"
        );
    }
}

#[test]
fn strength_changes_apply_without_a_reset() {
    let noise = white(3.0, 0.05, 3);
    let mut filter = DeepFilter::new(Strength::Gentle).unwrap();
    let (first, second) = noise.split_at(48_000 * 2);
    let gentle = run(&mut filter, first);
    filter.set_strength(Strength::Strong);
    assert_eq!(filter.strength(), Strength::Strong);
    let strong = run(&mut filter, second);
    let gentle_tail = 10.0 * (energy(&first[48_000..]) / energy(&gentle[48_000..])).log10();
    // Within 100 ms of the switch the floor drops well below the gentle cap.
    let strong_tail = 10.0 * (energy(&second[4_800..]) / energy(&strong[4_800..])).log10();
    assert!(
        strong_tail > gentle_tail + 15.0,
        "gentle {gentle_tail:.1} dB, strong {strong_tail:.1} dB"
    );
}

#[test]
fn filters_are_independent_across_threads() {
    // Upstream's `Clone` shared scratch tensors between copies; every filter
    // here must own its state, including when moved to another thread.
    let a = speech(1.0);
    let b = white(1.0, 0.1, 5);
    let reference = run(&mut DeepFilter::new(Strength::Strong).unwrap(), &a);
    let mut first = DeepFilter::new(Strength::Strong).unwrap();
    let mut second = DeepFilter::new(Strength::Strong).unwrap();
    let mut interleaved = Vec::new();
    for (x, y) in a.as_chunks::<HOP>().0.iter().zip(b.as_chunks::<HOP>().0) {
        let mut frame = *x;
        first.process(&mut frame).unwrap();
        interleaved.extend(frame);
        let mut frame = *y;
        second.process(&mut frame).unwrap();
    }
    assert_eq!(interleaved, reference);
    let moved =
        std::thread::spawn(move || run(&mut DeepFilter::new(Strength::Strong).unwrap(), &a))
            .join()
            .unwrap();
    assert_eq!(moved, reference);
}
