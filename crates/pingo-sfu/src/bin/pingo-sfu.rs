use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use axum::{Json, extract::Query, http::StatusCode, response::Html, routing::get};
use clap::Parser;
use pingo_proto::{MediaClaims, token};
use pingo_sfu::{Sfu, SfuConfig, VoiceHooks};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(about = "Pingo's standalone single-port WebRTC SFU")]
struct Args {
    #[arg(long, default_value = "0.0.0.0:7881")]
    http: SocketAddr,
    #[arg(long, default_value = "0.0.0.0:7882")]
    udp: SocketAddr,
    #[arg(long, value_delimiter = ',')]
    public_ip: Vec<IpAddr>,
    #[arg(long, default_value_t = 5)]
    max_forwarded_audio: usize,
    /// Enable unauthenticated token minting for the local interop page.
    #[arg(long)]
    dev_token: bool,
}

struct LoggingHooks;

#[async_trait]
impl VoiceHooks for LoggingHooks {
    async fn on_state(
        &self,
        room: &str,
        user: &str,
        muted: bool,
        deafened: bool,
        sharing_screen: bool,
        video: bool,
    ) {
        tracing::info!(
            room,
            user,
            muted,
            deafened,
            sharing_screen,
            video,
            "voice state"
        );
    }
    async fn on_leave(&self, room: &str, user: &str) {
        tracing::info!(room, user, "voice leave");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("pingo_sfu=info")),
        )
        .init();
    let args = Args::parse();
    let secret = std::env::var("PINGO_MEDIA_SECRET")
        .map_err(|_| "set PINGO_MEDIA_SECRET to the backend's media token secret")?;
    if args.dev_token && !args.http.ip().is_loopback() {
        return Err("--dev-token requires a loopback --http address".into());
    }
    let sfu = Sfu::start(
        SfuConfig {
            media_secret: secret.as_bytes().to_vec(),
            udp_bind: args.udp,
            public_ips: args.public_ip,
            max_forwarded_audio: args.max_forwarded_audio,
        },
        Arc::new(LoggingHooks),
    )
    .await?;
    let mut router = sfu
        .router::<()>()
        .route("/", get(|| async { Html(include_str!("../../dev.html")) }));
    if args.dev_token {
        router = router.route(
            "/dev-token",
            get(move |Query(query): Query<HashMap<String, String>>| {
                let secret = secret.clone();
                async move {
                    let room = query
                        .get("room")
                        .filter(|s| !s.is_empty() && s.len() <= 64)
                        .ok_or(StatusCode::BAD_REQUEST)?;
                    let user = query
                        .get("user")
                        .filter(|s| !s.is_empty() && s.len() <= 64)
                        .ok_or(StatusCode::BAD_REQUEST)?;
                    let claims = MediaClaims {
                        room: room.clone(),
                        user: user.clone(),
                        name: user.clone(),
                        color: "#f97316".into(),
                        can_speak: true,
                        can_video: true,
                        can_share: true,
                        exp: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as f64
                            + 60_000.0,
                    };
                    Ok::<_, StatusCode>(Json(
                        serde_json::json!({"token": token::sign(&claims, secret.as_bytes())}),
                    ))
                }
            }),
        );
        tracing::info!("local development token endpoint enabled");
    }
    let listener = tokio::net::TcpListener::bind(args.http).await?;
    tracing::info!(http = %listener.local_addr()?, udp = %sfu.udp_addr(), "Pingo SFU listening");
    // TODO: ICE-TCP and TURN deployment are outside the initial UDP-only transport.
    let shutdown_sfu = sfu.clone();
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            shutdown_sfu.shutdown().await;
        })
        .await?;
    sfu.shutdown().await;
    Ok(())
}
