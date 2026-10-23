use pingo_media::{
    FRAME_SAMPLES,
    audio::{
        apm::AudioProcessor,
        codec::{Decoder, Encoder, PacketDuration, Profile},
        denoise::{NoiseLevel, NoiseSuppressor, RnNoise},
        device::RateConverter,
        level::{Gate, VoiceActivity, dbov, rms},
        mixer::{Mixer, ParticipantMix},
    },
    pipeline::{AudioSettings, Quality, QualityReport, ReceivePipeline, SendPipeline},
};

fn tone<const N: usize>(hz: f32, amplitude: f32, offset: usize) -> [f32; N] {
    std::array::from_fn(|i| {
        amplitude * (std::f32::consts::TAU * hz * (i + offset) as f32 / 48_000.0).sin()
    })
}

#[test]
fn rfc6464_levels_and_vad_hangover() {
    assert_eq!(dbov(&[0.0; 480]), 127);
    assert_eq!(dbov(&[1.0; 480]), 0);
    assert_eq!(dbov(&tone::<480>(1000.0, 1.0, 0)), 3);
    let mut vad = VoiceActivity::default();
    vad.hangover_frames = 3;
    assert!(vad.detect(&tone::<480>(200.0, 0.1, 0)));
    for _ in 0..3 {
        assert!(vad.detect(&[0.0; 480]));
    }
    assert!(!vad.detect(&[0.0; 480]));
    let mut frame = [1.0; 480];
    vad.apply(&mut frame, true);
    assert!(frame[0] < 0.01 && frame[479] > 0.99);
    vad.apply(&mut frame, false);
    assert_eq!(frame[479], 0.0);
}

#[test]
fn apm_rejects_dc_and_stays_finite_on_speech_bursts() {
    let mut apm = AudioProcessor::new(false, false).unwrap();
    let mut frame = [0.3; 480];
    for _ in 0..100 {
        frame.fill(0.3);
        apm.capture(&mut frame).unwrap();
    }
    // At least 40 dB DC attenuation, including split-band filter roundoff.
    assert!(rms(&frame) < 0.003, "DC RMS {}", rms(&frame));
    for i in 0..100 {
        let mut frame = tone::<480>(250.0, if i % 20 < 10 { 0.03 } else { 0.0 }, i * 480);
        apm.render(&[0.0; 480]).unwrap();
        apm.capture(&mut frame).unwrap();
        assert!(frame.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    }
}

#[test]
fn rnnoise_scaling_suppression_and_off_bypass() {
    let mut suppressor = RnNoise::default();
    let original = tone::<480>(300.0, 0.1, 0);
    let mut frame = original;
    suppressor.process(&mut frame, NoiseLevel::Off);
    assert_eq!(frame, original);
    let mut seed = 71u64;
    let mut input_energy = 0.0;
    let mut output_energy = 0.0;
    for frame_index in 0..150 {
        let mut frame = std::array::from_fn(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            ((seed as u32 as f64 / u32::MAX as f64) as f32 - 0.5) * 0.04
        });
        let input = rms(&frame);
        let probability = suppressor.process(&mut frame, NoiseLevel::Strong);
        assert!((0.0..=1.0).contains(&probability));
        assert!(frame.iter().all(|v| v.is_finite() && v.abs() < 1.0));
        if frame_index > 50 {
            input_energy += input;
            output_energy += rms(&frame);
        }
    }
    assert!(
        output_energy < input_energy * 0.8,
        "noise input {input_energy} output {output_energy}"
    );
}

#[test]
fn opus_profiles_durations_plc_and_fec_roundtrip() {
    for profile in [Profile::Voice, Profile::Studio] {
        for duration in [PacketDuration::LowLatency10, PacketDuration::Default20] {
            let mut encoder = Encoder::new(profile, duration).unwrap();
            let mut decoder = Decoder::new(profile).unwrap();
            encoder.feedback(20.0).unwrap();
            let mut power = 0.0;
            for packet in 0..20 {
                let pcm: Vec<_> = (0..duration.samples())
                    .flat_map(|i| {
                        let left = (std::f32::consts::TAU
                            * 300.0
                            * (i + packet * duration.samples()) as f32
                            / 48000.0)
                            .sin()
                            * 0.2;
                        if profile == Profile::Voice {
                            vec![left]
                        } else {
                            vec![left, -left]
                        }
                    })
                    .collect();
                let payload = encoder.encode(&pcm).unwrap();
                let output = decoder
                    .decode(Some(&payload), duration.samples(), false)
                    .unwrap();
                assert_eq!(output.len(), pcm.len());
                power += rms(&output);
                if profile == Profile::Studio && packet > 2 {
                    let difference: f32 = output
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|s| (s[0] + s[1]).abs())
                        .sum();
                    assert!(difference < 10.0, "stereo must retain opposite phase");
                }
                if packet == 7 {
                    assert_eq!(
                        decoder
                            .decode(Some(&payload), duration.samples(), true)
                            .unwrap()
                            .len(),
                        pcm.len()
                    );
                }
            }
            assert!(power > 0.3);
            let plc = decoder.decode(None, duration.samples(), false).unwrap();
            assert_eq!(plc.len(), duration.samples() * profile.channels());
            assert!(plc.iter().all(|v| v.is_finite()));
        }
    }
}

#[test]
fn sinc_resamples_stereo_44100_and_96000_with_bounded_error() {
    for rate in [44_100, 96_000] {
        let mut converter = RateConverter::new(rate, 48_000, (rate / 100) as usize).unwrap();
        let mut output = Vec::new();
        for chunk in 0..100 {
            let input: Vec<_> = (0..rate / 100)
                .map(|i| {
                    let v = (std::f32::consts::TAU * 1000.0 * (chunk * (rate / 100) + i) as f32
                        / rate as f32)
                        .sin()
                        * 0.2;
                    [v, -v]
                })
                .collect();
            output.extend(converter.process(&input).unwrap());
        }
        assert!((output.len() as isize - 48_000).abs() < 200);
        assert!(output.iter().all(|s| (s[0] + s[1]).abs() < 1e-6));
        let power = (output.iter().skip(500).map(|s| s[0] * s[0]).sum::<f32>()
            / (output.len() - 500) as f32)
            .sqrt();
        assert!((power - 0.2 / 2.0f32.sqrt()).abs() < 0.005);
    }
}

#[test]
fn mixer_pan_volume_normalization_limiter_and_deafen() {
    let mut mixer = Mixer::default();
    let pcm = [0.1; 480];
    let dry = ParticipantMix {
        normalize: false,
        ..Default::default()
    };
    let hard_left = mixer.mix([(
        "one",
        pcm.as_slice(),
        1,
        ParticipantMix {
            pan: Some(-1.0),
            ..dry
        },
    )]);
    assert!(
        hard_left
            .iter()
            .all(|s| (s[0] - 0.1).abs() < 1e-6 && s[1].abs() < 1e-6)
    );
    let muted = mixer.mix([(
        "one",
        pcm.as_slice(),
        1,
        ParticipantMix { volume: 0.0, ..dry },
    )]);
    assert!(muted.iter().all(|s| *s == [0.0; 2]));
    let loud = [10.0; 480];
    let limited = mixer.mix([("one", loud.as_slice(), 1, dry)]);
    assert!(limited.iter().all(|s| s[0].abs() <= 1.0));
    let quiet = [0.02; 480];
    let mut output = [[0.0; 2]; 480];
    for _ in 0..1000 {
        output = mixer.mix([("quiet", quiet.as_slice(), 1, ParticipantMix::default())]);
    }
    assert!(output[479][0] > 0.05 && output[479][0] <= 0.081);
    mixer.deafened = true;
    assert!(
        mixer
            .mix([("one", pcm.as_slice(), 1, dry)])
            .iter()
            .all(|s| *s == [0.0; 2])
    );
}

#[test]
fn runtime_profile_gate_and_packet_duration_switches() {
    let mut settings = AudioSettings {
        noise_level: NoiseLevel::Off,
        agc: false,
        gate: Gate::Open,
        ..Default::default()
    };
    let mut send = SendPipeline::new(settings.clone()).unwrap();
    assert!(
        send.process_mono(&tone::<480>(300.0, 0.2, 0))
            .unwrap()
            .is_none()
    );
    let packet = send
        .process_mono(&tone::<480>(300.0, 0.2, 480))
        .unwrap()
        .unwrap();
    assert_eq!(packet.timestamp, 0);
    assert!(packet.audio_level < 30);
    settings.packet_duration = PacketDuration::LowLatency10;
    settings.profile = Profile::Studio;
    send.set_settings(settings.clone()).unwrap();
    let packet = send
        .process(&tone::<480>(400.0, 0.2, 960).map(|x| [x, -x]))
        .unwrap()
        .unwrap();
    assert_eq!(packet.timestamp, 960);
    assert_eq!(
        opus::packet::get_nb_samples(&packet.payload, 48000).unwrap(),
        480
    );
    settings.gate = Gate::PushToTalk { pressed: false };
    send.set_settings(settings).unwrap();
    for _ in 0..5 {
        send.process(&[[0.2; 2]; 480]).unwrap();
    }
    let packet = send.process(&[[0.2; 2]; 480]).unwrap().unwrap();
    assert_eq!(packet.audio_level, 127);
    assert!(!packet.speaking);
}

#[test]
fn receive_mix_runtime_volume_and_quality_labels() {
    let settings = AudioSettings {
        noise_level: NoiseLevel::Off,
        gate: Gate::Open,
        ..Default::default()
    };
    let mut send = SendPipeline::new(settings.clone()).unwrap();
    let mut receive = ReceivePipeline::new(settings.clone());
    receive.add_participant("one".into()).unwrap();
    let mut power = 0.0;
    for i in 0..80 {
        if let Some(packet) = send
            .process_mono(&tone::<480>(300.0, 0.2, i * 480))
            .unwrap()
        {
            receive.push("one", packet, i as f64 * 10.0).unwrap();
        }
        power += receive
            .render(i as f64 * 10.0)
            .unwrap()
            .iter()
            .map(|s| s[0].abs())
            .sum::<f32>();
    }
    assert!(power > 20.0);
    let mut settings = settings;
    settings.deafened = true;
    receive.set_settings(settings);
    assert!(
        receive
            .render(800.0)
            .unwrap()
            .iter()
            .all(|s| *s == [0.0; 2])
    );
    assert_eq!(
        QualityReport::new(0.1, 5.0, 30.0, 30.0).quality,
        Quality::Great
    );
    assert_eq!(
        QualityReport::new(5.0, 25.0, 70.0, 50.0).quality,
        Quality::Ok
    );
    assert_eq!(
        QualityReport::new(1.0, 5.0, 300.0, 30.0).quality,
        Quality::Poor
    );
    assert_eq!(
        QualityReport::new(f32::NAN, 5.0, 30.0, 30.0).quality,
        Quality::Poor
    );
    assert_eq!(FRAME_SAMPLES, 480);
}

#[cfg(feature = "deepfilter")]
#[test]
fn deepfilter_stub_is_explicit() {
    assert!(pingo_media::audio::denoise::deepfilter().is_err());
}
