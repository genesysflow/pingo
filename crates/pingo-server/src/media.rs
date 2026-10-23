//! RxFerrix owns the runtime; the first media request hands us its running App.
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use pingo_sfu::{MediaSecretProvider, Sfu, SfuConfig, VoiceHooks};
use rxferrix::App;
use tokio::sync::OnceCell;
use tower::ServiceExt;

use crate::voice;

#[derive(Clone)]
pub struct Media {
    config: SfuConfig,
    sfu: Arc<OnceCell<Sfu>>,
}

impl Media {
    pub fn new(config: SfuConfig) -> Self {
        Self {
            config,
            sfu: Arc::new(OnceCell::new()),
        }
    }

    pub fn from_env() -> rxferrix::Result<Self> {
        let port = std::env::var("PINGO_UDP_PORT")
            .unwrap_or_else(|_| "7882".into())
            .parse::<u16>()?;
        let ips = std::env::var("PINGO_PUBLIC_IPS").unwrap_or_default();
        let public_ips: Vec<IpAddr> = ips
            .split(',')
            .map(str::trim)
            .filter(|ip| !ip.is_empty())
            .map(str::parse)
            .collect::<Result<_, _>>()?;
        let bind_ip = if public_ips.first().is_some_and(IpAddr::is_ipv6) {
            IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED)
        } else {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        };
        Ok(Self::new(SfuConfig {
            udp_bind: SocketAddr::new(bind_ip, port),
            public_ips,
            ..Default::default()
        }))
    }

    async fn start(&self, app: App) -> rxferrix::Result<&Sfu> {
        self.sfu
            .get_or_try_init(|| async {
                // No new session can race this sweep. Multiple initial HTTP requests
                // share the cell; failed initialization can be retried by the next one.
                while app.mutation::<voice::reset>(()).await? == 500 {}
                let backend = Arc::new(Backend(app.clone()));
                let sfu =
                    Sfu::start_with_secret_provider(self.config.clone(), backend.clone(), backend)
                        .await?;
                tracing::info!(udp = %sfu.udp_addr(), "media server started");
                let shutdown = sfu.clone();
                tokio::spawn(async move {
                    app.closed().await;
                    shutdown.shutdown().await;
                });
                Ok(sfu)
            })
            .await
    }

    pub fn router(self) -> Router<App> {
        Router::new().route(
            "/rtc",
            get(move |State(app): State<App>, request: Request| {
                let media = self.clone();
                async move { media.request(app, request).await }
            }),
        )
    }

    async fn request(&self, app: App, request: Request) -> Response {
        match self.start(app).await {
            Ok(sfu) => sfu.router::<()>().oneshot(request).await.unwrap(),
            Err(error) => {
                tracing::error!(%error, "cannot start media server");
                (StatusCode::SERVICE_UNAVAILABLE, "media unavailable").into_response()
            }
        }
    }
}

struct Backend(App);

#[async_trait]
impl MediaSecretProvider for Backend {
    async fn media_secret(&self) -> Option<Vec<u8>> {
        match self.0.query::<voice::admission_secret>(()).await {
            Ok(secret) => Some(secret.into_bytes()),
            Err(_) => {
                // Do not log query errors: a secret must never appear in logs.
                tracing::warn!("cannot read PINGO_MEDIA_SECRET for admission");
                None
            }
        }
    }
}

impl Backend {
    // The SFU serializes hooks on a separate worker with a five-second budget.
    // Bound all attempts together so retries preserve delivery order and cannot
    // outlive the callback and overwrite a later session's state.
    async fn persist<F, Fut>(&self, mut write: F)
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = rxferrix::Result<()>>,
    {
        let result = tokio::time::timeout(Duration::from_secs(4), async {
            for attempt in 0..3 {
                match write().await {
                    Ok(()) => return,
                    Err(error) => tracing::warn!(attempt, %error, "voice hook write failed"),
                }
                if attempt < 2 {
                    tokio::time::sleep(Duration::from_millis(100 << attempt)).await;
                }
            }
            tracing::error!("voice hook exhausted retries");
        })
        .await;
        if result.is_err() {
            tracing::error!("voice hook retry budget expired");
        }
    }
}

#[async_trait]
impl VoiceHooks for Backend {
    async fn on_state(
        &self,
        room: &str,
        user: &str,
        muted: bool,
        deafened: bool,
        sharing_screen: bool,
        video: bool,
    ) {
        let (Ok(room), Ok(user)) = (room.parse(), user.parse()) else {
            tracing::error!("SFU supplied invalid voice identifiers");
            return;
        };
        self.persist(|| {
            self.0.mutation::<voice::set_state>((
                room,
                user,
                muted,
                deafened,
                sharing_screen,
                video,
            ))
        })
        .await;
    }

    async fn on_leave(&self, room: &str, user: &str) {
        let (Ok(room), Ok(user)) = (room.parse(), user.parse()) else {
            tracing::error!("SFU supplied invalid voice identifiers");
            return;
        };
        self.persist(|| self.0.mutation::<voice::left>((room, user)))
            .await;
    }
}

#[cfg(test)]
mod tests;
