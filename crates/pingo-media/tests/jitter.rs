use pingo_media::audio::{
    codec::{EncodedPacket, Encoder, PacketDuration, Profile},
    jitter::JitterBuffer,
    level::rms,
};

fn packets(count: usize) -> Vec<EncodedPacket> {
    let mut encoder = Encoder::new(Profile::Voice, PacketDuration::Default20).unwrap();
    (0..count)
        .map(|n| {
            let samples: Vec<_> = (0..960)
                .map(|i| {
                    let t = (n * 960 + i) as f32 / 48000.0;
                    // Varying harmonics and pauses exercise speech/quiet stretch decisions.
                    let envelope = if n % 40 < 30 { 0.15 } else { 0.005 };
                    envelope
                        * ((std::f32::consts::TAU * 180.0 * t).sin()
                            + 0.3 * (std::f32::consts::TAU * 430.0 * t).sin())
                })
                .collect();
            EncodedPacket {
                sequence: n as u16,
                timestamp: (n * 960) as u32,
                duration: PacketDuration::Default20,
                payload: encoder.encode(&samples).unwrap(),
                audio_level: 20,
                speaking: true,
            }
        })
        .collect()
}
fn random(seed: &mut u64) -> f64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    (*seed as u32) as f64 / u32::MAX as f64
}

#[test]
fn reorders_deduplicates_and_extends_rtp_wrap() {
    let mut b = JitterBuffer::new(Profile::Voice).unwrap();
    let mut packets = packets(8);
    for (i, p) in packets.iter_mut().enumerate() {
        p.sequence = 65533u16.wrapping_add(i as u16);
        p.timestamp = (u32::MAX - 1919).wrapping_add((i * 960) as u32);
    }
    for index in [1, 0, 3, 2, 2, 4, 5, 6, 7] {
        b.push(packets[index].clone(), index as f64 * 20.0).unwrap();
    }
    assert_eq!(b.buffered_packets(), 8);
    for tick in 0..18 {
        let output = b.render(tick as f64 * 10.0).unwrap();
        assert_eq!(output.len(), 480);
        assert!(output.iter().all(|s| s.is_finite()));
    }
    assert_eq!(b.stats().duplicates, 1);
    assert_eq!(b.stats().lost_packets, 0);
    assert_eq!(b.buffered_packets(), 0);
}

#[test]
fn next_packet_fec_is_not_consumed_and_bursts_use_plc() {
    let mut b = JitterBuffer::new(Profile::Voice).unwrap();
    let packets = packets(20);
    // Loss of 1 has next-packet FEC, loss burst 4..7 requires PLC then FEC.
    for (i, p) in packets.iter().enumerate() {
        if i != 1 && !(4..=7).contains(&i) {
            b.push(p.clone(), i as f64 * 20.0).unwrap();
        }
    }
    for tick in 0..42 {
        b.render(tick as f64 * 10.0).unwrap();
    }
    let stats = b.stats();
    assert_eq!(stats.lost_packets, 5);
    assert_eq!(stats.fec_attempts, 2);
    assert_eq!(stats.plc_packets, 3);
    assert_eq!(stats.played_packets, 20);
    assert_eq!(stats.loss_percent, 25.0);
}

#[test]
fn seeded_network_has_continuous_bounded_output_and_recovers() {
    let packets = packets(1500);
    let mut seed = 0x1a93f1u64;
    let mut arrivals = Vec::new();
    for (i, p) in packets.iter().enumerate() {
        let impaired = (200..700).contains(&i);
        if impaired && ((i % 101 >= 25 && i % 101 <= 28) || random(&mut seed) < 0.05) {
            continue;
        }
        let jitter = if impaired {
            random(&mut seed) * 65.0
        } else {
            random(&mut seed) * 2.0
        };
        let arrival = i as f64 * 20.0 + 20.0 + jitter;
        arrivals.push((arrival, p.clone()));
        if impaired && random(&mut seed) < 0.15 {
            arrivals.push((arrival + 3.0, p.clone()));
        }
    }
    arrivals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut arrivals = arrivals.into_iter().peekable();
    let mut b = JitterBuffer::new(Profile::Voice).unwrap();
    let mut final_power = 0.0;
    for tick in 0..3000 {
        let now = tick as f64 * 10.0;
        while arrivals.peek().is_some_and(|p| p.0 <= now) {
            let (arrival, p) = arrivals.next().unwrap();
            b.push(p, arrival).unwrap();
        }
        let output = b.render(now).unwrap();
        assert_eq!(output.len(), 480, "playout gap at {now}");
        assert!(output.iter().all(|s| s.is_finite() && s.abs() <= 1.1));
        let stats = b.stats();
        assert!((20.0..=200.0).contains(&stats.target_delay_ms));
        assert!(stats.current_delay_ms <= 210.0, "{now}: {stats:?}");
        assert!(b.buffered_packets() <= 64);
        if tick > 2800 {
            final_power += rms(&output);
        }
    }
    let stats = b.stats();
    assert!(
        stats.duplicates > 20
            && stats.lost_packets > 20
            && stats.plc_packets > 0
            && stats.fec_attempts > 0,
        "{stats:?}"
    );
    assert!(
        stats.expanded_samples > 0 && stats.compressed_samples > 0,
        "{stats:?}"
    );
    assert!(
        stats.target_delay_ms < 30.0,
        "must recover when jitter subsides: {stats:?}"
    );
    assert!(
        stats.loss_percent < 10.0 && stats.late_percent < 10.0,
        "{stats:?}"
    );
    assert!(
        final_power > 1.0,
        "decoder must recover audible signal after bursts"
    );
}

#[test]
fn sender_clock_drift_does_not_accumulate_unbounded_latency() {
    let payload = packets(1).remove(0);
    for drift in [-0.0015, 0.0015] {
        let mut b = JitterBuffer::new(Profile::Voice).unwrap();
        let mut index = 0;
        for tick in 0..6000 {
            let now = tick as f64 * 10.0;
            while index as f64 * 20.0 * (1.0 + drift) <= now {
                let mut p = payload.clone();
                p.sequence = index as u16;
                p.timestamp = index * 960;
                b.push(p, index as f64 * 20.0 * (1.0 + drift)).unwrap();
                index += 1;
            }
            assert_eq!(b.render(now).unwrap().len(), 480);
            assert!(
                b.stats().current_delay_ms <= 210.0,
                "drift {drift}: {:?}",
                b.stats()
            );
        }
        assert!(
            b.stats().loss_percent < 2.0,
            "drift {drift}: {:?}",
            b.stats()
        );
        assert!(b.stats().current_delay_ms < 100.0, "{:?}", b.stats());
    }
}

#[test]
fn rejects_late_malformed_and_far_future_packets_and_bounds_queue() {
    let packets = packets(100);
    let mut b = JitterBuffer::new(Profile::Voice).unwrap();
    b.push(packets[0].clone(), 0.0).unwrap();
    for tick in 0..10 {
        b.render(tick as f64 * 10.0).unwrap();
    }
    assert!(!b.push(packets[1].clone(), 100.0).unwrap());
    assert!(b.stats().late > 0);
    let mut bad = packets[4].clone();
    bad.sequence = 1000;
    assert!(b.push(bad, 100.0).is_err());
    let mut bad = packets[4].clone();
    bad.payload.clear();
    assert!(b.push(bad, 100.0).is_err());
    let mut bad = packets[4].clone();
    bad.duration = PacketDuration::LowLatency10;
    assert!(b.push(bad, 100.0).is_err());
    assert!(b.push(packets[4].clone(), f64::NAN).is_err());
    assert!(b.render(f64::NAN).is_err());
    let mut b = JitterBuffer::new(Profile::Voice).unwrap();
    for (i, packet) in packets.into_iter().enumerate() {
        b.push(packet, i as f64 * 20.0).unwrap();
    }
    assert_eq!(b.buffered_packets(), 64);
    b.render(2000.0).unwrap();
    assert!(b.stats().resets > 0);
    assert!(b.stats().current_delay_ms <= 200.0);
}

#[test]
fn suspension_without_new_packets_reanchors_expired_audio() {
    let mut b = JitterBuffer::new(Profile::Voice).unwrap();
    b.push(packets(1).remove(0), 0.0).unwrap();
    b.render(20.0).unwrap();
    assert_eq!(b.render(5000.0).unwrap().len(), 480);
    assert!(b.stats().current_delay_ms <= 200.0);
    assert_eq!(b.stats().resets, 1);
}

#[test]
fn duration_changes_on_lost_packet_follow_rtp_timestamps() {
    let mut b = JitterBuffer::new(Profile::Voice).unwrap();
    let packets = packets(1);
    b.push(packets[0].clone(), 0.0).unwrap();
    let mut encoder = Encoder::new(Profile::Voice, PacketDuration::LowLatency10).unwrap();
    for i in 0..30 {
        let payload = encoder.encode(&[0.1; 480]).unwrap();
        let p = EncodedPacket {
            sequence: i + 1,
            timestamp: 960 + i as u32 * 480,
            duration: PacketDuration::LowLatency10,
            payload,
            audio_level: 20,
            speaking: true,
        };
        if i != 0 {
            b.push(p, 20.0 + i as f64 * 10.0).unwrap();
        }
    }
    for tick in 0..34 {
        assert_eq!(b.render(tick as f64 * 10.0).unwrap().len(), 480);
    }
    assert_eq!(b.stats().lost_packets, 1, "{:?}", b.stats());
    assert_eq!(b.stats().fec_attempts, 1);
    assert!(b.stats().current_delay_ms < 40.0);
}

#[test]
fn corrupt_opus_payload_cannot_stop_the_playout_clock() {
    let mut b = JitterBuffer::new(Profile::Voice).unwrap();
    // Valid 20 ms TOC but truncated VBR frame-length header.
    let packet = EncodedPacket {
        sequence: 0,
        timestamp: 0,
        duration: PacketDuration::Default20,
        payload: vec![0x02, 0xff],
        audio_level: 20,
        speaking: true,
    };
    b.push(packet, 0.0).unwrap();
    for tick in 0..10 {
        let output = b.render(tick as f64 * 10.0).unwrap();
        assert_eq!(output.len(), 480);
        assert!(output.iter().all(|s| s.is_finite()));
    }
    assert_eq!(b.stats().corrupt_packets, 1);
}
