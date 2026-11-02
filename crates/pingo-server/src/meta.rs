//! Publishes the server's version so desktop apps, which update separately, can tell whether they still fit.
//! A plain route rather than an RxFerrix query: apps read it before connecting or signing in.
use axum::routing::get;
use axum::{Json, Router};
use pingo_proto::VersionInfo;
use pingo_proto::version::VERSION_PATH;
use rxferrix::App;

pub fn router() -> Router<App> {
    Router::new().route(
        VERSION_PATH,
        get(|| async { Json(VersionInfo::current(env!("CARGO_PKG_VERSION"))) }),
    )
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use rxferrix::testing::TestApp;
    use tower::ServiceExt;

    #[tokio::test]
    async fn version_is_public_and_matches_the_shared_protocol() {
        let t = TestApp::new().await.unwrap();
        let response = super::router()
            .with_state(t.app().clone())
            .oneshot(
                Request::get("/.well-known/pingo")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1 << 16).await.unwrap();
        let info: pingo_proto::VersionInfo = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            info,
            pingo_proto::VersionInfo::current(env!("CARGO_PKG_VERSION"))
        );
    }
}
