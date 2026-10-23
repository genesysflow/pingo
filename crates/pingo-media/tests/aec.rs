#![cfg(feature = "webrtc-apm")]

use pingo_media::{FRAME_SAMPLES, audio::apm::AudioProcessor};

/// A deterministic voiced source with syllabic modulation and breath noise.
fn speech(seconds: usize) -> Vec<f32> {
    let mut seed = 19u64;
    let mut breath = 0.0;
    (0..seconds * 48_000)
        .map(|i| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            breath += 0.12 * ((seed as u32 as f32 / u32::MAX as f32 - 0.5) - breath);
            let t = i as f32 / 48_000.0;
            let envelope = 0.15 + 0.85 * (std::f32::consts::TAU * 2.7 * t).sin().powi(2);
            envelope
                * ((1..=7)
                    .map(|h| (std::f32::consts::TAU * 137.0 * h as f32 * t).sin() * 0.13 / h as f32)
                    .sum::<f32>()
                    + breath * 0.1)
        })
        .collect()
}

#[test]
fn aec3_reduces_delayed_room_echo_and_preserves_double_talk() {
    let far = speech(20);
    let mut apm = AudioProcessor::new(false, true).unwrap();
    assert!(apm.has_aec());
    apm.set_stream_delay_ms(Some(40));
    let mut echo_power = 0.0f64;
    let mut residual_power = 0.0f64;
    let mut near_sin = 0.0f64;
    let mut near_cos = 0.0f64;
    let mut near_count = 0;
    for frame in 0..2000 {
        let start = frame * FRAME_SAMPLES;
        let render = std::array::from_fn(|i| far[start + i]);
        let echo: [f32; FRAME_SAMPLES] = std::array::from_fn(|i| {
            let n = start + i;
            // Direct path and two short room reflections, plus low-pass coloration.
            [(1920, 0.38), (1921, 0.16), (2117, 0.12), (2441, -0.07)]
                .into_iter()
                .map(|(delay, gain)| n.checked_sub(delay).map_or(0.0, |k| far[k] * gain))
                .sum()
        });
        let mut capture = std::array::from_fn(|i| {
            let t = (start + i) as f32 / 48_000.0;
            echo[i]
                + if frame >= 1600 {
                    0.08 * (std::f32::consts::TAU * 2011.0 * t).sin()
                } else {
                    0.0
                }
        });
        // Unequal acoustic coupling from antiphase speakers must still cancel:
        // averaging the stereo reference to mono would erase it completely.
        apm.render_stereo(&render.map(|s| [s, -s])).unwrap();
        apm.capture(&mut capture).unwrap();
        if (1200..1600).contains(&frame) {
            echo_power += echo.iter().map(|x| (*x as f64).powi(2)).sum::<f64>();
            residual_power += capture.iter().map(|x| (*x as f64).powi(2)).sum::<f64>();
        }
        if frame >= 1800 {
            for (i, sample) in capture.iter().enumerate() {
                let phase = std::f64::consts::TAU * 2011.0 * (start + i) as f64 / 48_000.0;
                near_sin += *sample as f64 * phase.sin();
                near_cos += *sample as f64 * phase.cos();
                near_count += 1;
            }
        }
    }
    let erle = 10.0 * (echo_power / residual_power).log10();
    let near_gain = 2.0 * near_sin.hypot(near_cos) / near_count as f64 / 0.08;
    println!("AEC3: ERLE {erle:.2} dB; double-talk near-end gain {near_gain:.3}");
    // 15 dB means >96.8% echo-power removal; comfortably below typical AEC3
    // steady-state performance, allowing platform differences and comfort noise.
    assert!(erle >= 15.0, "ERLE {erle:.2} dB");
    // A mute-all processor cannot pass: independent near-end speech must survive
    // simultaneous far-end playback within 6 dB (phase-independent projection).
    assert!((0.5..1.5).contains(&near_gain), "near-end gain {near_gain}");
}
