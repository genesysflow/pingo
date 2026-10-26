//! Test-only host for the real backend, which currently has a binary target.
//! Import production modules rather than copying admission, schema or SFU hooks.
//! Keep tests disabled on this helper to avoid recursively importing server tests.
// Clippy --all-targets also compiles a test target despite lib.test = false.
#![cfg(not(test))]

#[path = "../../../pingo-server/src/access.rs"]
pub mod access;
#[path = "../../../pingo-server/src/dms.rs"]
pub mod dms;
#[path = "../../../pingo-server/src/files.rs"]
pub mod files;
#[path = "../../../pingo-server/src/inbox.rs"]
pub mod inbox;
#[path = "../../../pingo-server/src/invites.rs"]
pub mod invites;
#[path = "../../../pingo-server/src/media.rs"]
pub mod media;
#[path = "../../../pingo-server/src/messages.rs"]
pub mod messages;
#[path = "../../../pingo-server/src/model.rs"]
pub mod model;
#[path = "../../../pingo-server/src/presence.rs"]
pub mod presence;
#[path = "../../../pingo-server/src/profiles.rs"]
pub mod profiles;
#[path = "../../../pingo-server/src/reactions.rs"]
pub mod reactions;
#[path = "../../../pingo-server/src/rooms.rs"]
pub mod rooms;
#[path = "../../../pingo-server/src/spaces.rs"]
pub mod spaces;
#[path = "../../../pingo-server/src/typing.rs"]
pub mod typing;
#[path = "../../../pingo-server/src/voice.rs"]
pub mod voice;

use rxferrix::{Id, auth::User, testing::TestApp};
use serde_json::{Value, json};
use std::{net::SocketAddr, time::Duration};

pub struct Server {
    app: TestApp,
    http: tokio::task::JoinHandle<()>,
    pub url: String,
    room: Id<model::Room>,
    users: [Id<User>; 2],
    auth: [String; 2],
}
impl Server {
    pub async fn start() -> Self {
        let app = TestApp::with_options(rxferrix::AppOptions {
            dev: true,
            ..Default::default()
        })
        .await
        .unwrap();
        let a = app
            .create_user("publisher@native-video.test", None)
            .await
            .unwrap();
        let b = app
            .create_user("viewer@native-video.test", None)
            .await
            .unwrap();
        let space = app
            .as_user(a)
            .mutation::<spaces::create>(("Native video".into(), None))
            .await
            .unwrap();
        let invite = app
            .as_user(a)
            .mutation::<invites::create>((space, None, None))
            .await
            .unwrap();
        app.as_user(b)
            .mutation::<invites::accept>((invite,))
            .await
            .unwrap();
        let room = app.as_user(a).query::<rooms::list>((space,)).await.unwrap()[0].id;
        let mut auth = Vec::new();
        for user in [a, b] {
            auth.push(
                app.app()
                    .auth()
                    .sign_in_as(app.app(), user)
                    .await
                    .unwrap()
                    .access_token,
            );
        }
        let media = media::Media::new(pingo_sfu::SfuConfig {
            udp_bind: "127.0.0.1:0".parse().unwrap(),
            public_ips: vec!["127.0.0.1".parse().unwrap()],
            ..Default::default()
        });
        let router = rxferrix::ops::web::router(
            app.app().clone(),
            &rxferrix::ops::WebOptions {
                routes: Some(media.router()),
                ..Default::default()
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let http = tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        Self {
            app,
            http,
            url: format!("ws://{address}/rtc"),
            room,
            users: [a, b],
            auth: auth.try_into().unwrap(),
        }
    }
    pub async fn tokens(&self) -> [String; 2] {
        let mut tokens = Vec::new();
        for auth in &self.auth {
            let reply: Value = reqwest::Client::new()
                .post(
                    self.url
                        .replace("ws://", "http://")
                        .replace("/rtc", "/api/mutation"),
                )
                .bearer_auth(auth)
                .json(&json!({"path":"voice:join","args":{"room":self.room}}))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(reply["status"], "success", "{reply}");
            tokens.push(reply["value"]["token"].as_str().unwrap().to_owned());
        }
        tokens.try_into().unwrap()
    }
    pub async fn assert_empty(&self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if self
                    .app
                    .as_user(self.users[0])
                    .query::<voice::participants>((self.room,))
                    .await
                    .unwrap()
                    .is_empty()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("SFU leave hooks must clear backend voice state");
    }
    pub async fn stop(self) {
        self.assert_empty().await;
        self.app.app().begin_close();
        self.http.abort();
        let _ = self.http.await;
    }
}
