use pingo_media::{
    audio::device::DeviceIo,
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
        println!("join --url ws://host/rtc --token TOKEN [--input NAME] [--output NAME]");
        return Ok(());
    }
    let url = argument("--url").ok_or_else(|| anyhow::anyhow!("--url is required"))?;
    let token = argument("--token").ok_or_else(|| anyhow::anyhow!("--token is required"))?;
    let mut config = ClientConfig::new(url, token);
    config.settings.input_device = argument("--input");
    config.settings.output_device = argument("--output");
    let io = DeviceIo::open(
        config.settings.input_device.clone(),
        config.settings.output_device.clone(),
    )?;
    let client = Client::connect(config).await?;
    let mut events = client.events.subscribe();
    let (commands, receiver) = mpsc::channel(32);
    let run = client.run(Some(io), receiver);
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
        }
    }
    Ok(())
}
