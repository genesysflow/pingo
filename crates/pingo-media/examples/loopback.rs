//! Sound check: native mic -> send DSP/Opus -> simulated network -> jitter/mix -> speakers.
mod support;
use pingo_media::{
    audio::{
        codec::{PacketDuration, Profile},
        device::DeviceIo,
        level::Gate,
    },
    pipeline::{AudioSettings, QualityReport, ReceivePipeline, SendPipeline},
};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if support::flag("--help") {
        println!(
            "loopback [--loss 5] [--jitter 30] [--seconds 30] [--input NAME] [--output NAME] [--studio] [--low-latency]"
        );
        return Ok(());
    }
    let loss = support::number("--loss", 0.0)?.min(100.0);
    let jitter = support::number("--jitter", 0.0)?.min(200.0);
    let seconds = support::number("--seconds", 30.0)?;
    let settings = AudioSettings {
        input_device: support::value("--input"),
        output_device: support::value("--output"),
        gate: Gate::Open,
        profile: if support::flag("--studio") {
            Profile::Studio
        } else {
            Profile::Voice
        },
        packet_duration: if support::flag("--low-latency") {
            PacketDuration::LowLatency10
        } else {
            PacketDuration::Default20
        },
        ..Default::default()
    };
    let mut io = DeviceIo::open(
        settings.input_device.clone(),
        settings.output_device.clone(),
    )?;
    let mut sender = SendPipeline::new(settings.clone())?;
    sender.feedback(loss as f32)?;
    let mut receiver = ReceivePipeline::new(settings);
    receiver.add_participant("loopback".into())?;
    println!(
        "Algorithmic budget at 20 ms jitter: {:.1} ms; AEC available: {}",
        sender.latency_budget_ms(20.0)?,
        sender.has_aec()
    );
    let start = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut network = VecDeque::new();
    let mut seed = 0x73af19;
    let mut last_report = 0;
    loop {
        tokio::select! {_=tick.tick()=>{},_=tokio::signal::ctrl_c()=>break}
        let now = start.elapsed().as_secs_f64() * 1000.0;
        if now >= seconds * 1000.0 {
            break;
        }
        if io.stats().fault {
            match io.recover() {
                Ok(true) => println!("Reopened audio devices"),
                Ok(false) => continue,
                Err(error) => {
                    eprintln!("audio device unavailable: {error}");
                    continue;
                }
            }
        }
        // Drain capture backlog, without ever doing DSP in a CPAL callback.
        while let Some(frame) = io.capture()? {
            if let Some(packet) = sender.process(&frame)?
                && support::random(&mut seed) * 100.0 >= loss
            {
                let arrival = now + support::random(&mut seed) * jitter;
                let index = network
                    .iter()
                    .position(|(t, _)| *t > arrival)
                    .unwrap_or(network.len());
                network.insert(index, (arrival, packet));
            }
        }
        while network.front().is_some_and(|(t, _)| *t <= now) {
            let (arrival, packet) = network.pop_front().unwrap();
            receiver.push("loopback", packet, arrival)?;
        }
        let output = receiver.render(now)?;
        if !io.playback(&output)? {
            eprintln!("playback queue full");
        }
        sender.render_reference(&output)?;
        let second = (now / 1000.0) as u64;
        if second > last_report {
            last_report = second;
            let stats = receiver.stats()["loopback"];
            println!(
                "{}; concealed {:.1}%, late {:.1}%; {:?}",
                QualityReport::from_jitter(stats, 0.0),
                stats.concealed_percent,
                stats.late_percent,
                io.stats()
            );
        }
    }
    Ok(())
}
