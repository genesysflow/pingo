use pingo_media::{
    audio::{codec::Profile, device::DeviceIo},
    rtc::{Client, ClientConfig, Command},
};
use tokio::sync::{broadcast, mpsc};

fn argument(name: &str) -> Option<String> {
    let args: Vec<_> = std::env::args().collect();
    args.windows(2).find(|v| v[0] == name).map(|v| v[1].clone())
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().any(|arg| arg == "--help") {
        println!(
            "join --url ws://host/rtc --token TOKEN [--input NAME] [--output NAME] [--studio] [--headless] [--seconds N] (or set PINGO_MEDIA_TOKEN)"
        );
        return Ok(());
    }
    let url = argument("--url").ok_or_else(|| anyhow::anyhow!("--url is required"))?;
    let token = argument("--token")
        .or_else(|| std::env::var("PINGO_MEDIA_TOKEN").ok())
        .ok_or_else(|| anyhow::anyhow!("--token or PINGO_MEDIA_TOKEN is required"))?;
    let seconds = argument("--seconds")
        .map(|s| s.parse::<u64>())
        .transpose()?;
    let headless = std::env::args().any(|arg| arg == "--headless");
    let mut config = ClientConfig::new(url, token);
    if std::env::args().any(|arg| arg == "--studio") {
        config.settings.profile = Profile::Studio;
    }
    config.settings.input_device = argument("--input");
    config.settings.output_device = argument("--output");
    let io = if headless {
        None
    } else {
        Some(DeviceIo::open(
            config.settings.input_device.clone(),
            config.settings.output_device.clone(),
        )?)
    };
    let client = Client::connect(config).await?;
    let mut events = client.events.subscribe();
    let (commands, receiver) = mpsc::channel(32);
    let synthetic = headless.then(|| {
        let tx = commands.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(10));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut offset = 0usize;
            loop {
                tick.tick().await;
                let frame = std::array::from_fn(|i| {
                    let t = (offset + i) as f32 / 48_000.0;
                    let sample = 0.12 * (std::f32::consts::TAU * 440.0 * t).sin();
                    [sample, sample]
                });
                offset += pingo_media::FRAME_SAMPLES;
                if tx.send(Command::Capture(Box::new(frame))).await.is_err() {
                    break;
                }
            }
        })
    });
    let end = async move {
        if let Some(seconds) = seconds {
            tokio::time::sleep(std::time::Duration::from_secs(seconds)).await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    tokio::pin!(end);
    let run = client.run(io, receiver);
    tokio::pin!(run);
    let mut leaving = false;
    loop {
        tokio::select! {
            result=&mut run=>{result?;break;}
            event=events.recv()=>match event {
                Ok(event)=>println!("{event:?}"),
                Err(broadcast::error::RecvError::Lagged(count))=>eprintln!("UI missed {count} events"),
                Err(broadcast::error::RecvError::Closed)=>break,
            },
            _=tokio::signal::ctrl_c(),if !leaving=>{leaving=true;commands.send(Command::Leave).await?;}
            _=&mut end,if !leaving=>{leaving=true;commands.send(Command::Leave).await?;}
        }
    }
    if let Some(task) = synthetic {
        task.abort();
    }
    Ok(())
}
