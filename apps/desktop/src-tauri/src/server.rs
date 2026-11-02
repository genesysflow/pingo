//! Checks that a server answers and speaks a protocol this app supports, before the UI commits to it.
//! Runs natively because custom server routes sit outside RxFerrix's CORS handling.
use std::time::Duration;

use pingo_proto::version::{Compatibility, VERSION_PATH, VersionInfo, compatibility};
use serde::Serialize;

use crate::config;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCheck {
    pub origin: String,
    pub server: VersionInfo,
    pub app: VersionInfo,
    pub compatibility: Compatibility,
}

/// Servers from before the version endpoint answer its path with the web app's HTML, so anything that
/// isn't a version document means "ask the older way".
pub fn parse_version(success: bool, body: &[u8]) -> Option<VersionInfo> {
    success.then(|| serde_json::from_slice(body).ok()).flatten()
}

#[tauri::command]
pub async fn desktop_server_check(
    app: tauri::AppHandle,
    server: String,
) -> Result<ServerCheck, String> {
    let origin = config::server_origin(&server)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;
    let unreachable =
        |_| format!("Couldn’t reach {origin}. Check the address and that the server is running.");
    let not_pingo = || format!("{origin} answered, but it doesn’t look like a Pingo server.");
    let response = client
        .get(format!("{origin}{VERSION_PATH}"))
        .send()
        .await
        .map_err(unreachable)?;
    let success = response.status().is_success();
    let body = response.bytes().await.map_err(unreachable)?;
    let server = match parse_version(success, &body) {
        Some(info) => info,
        None => {
            let providers = client
                .get(format!("{origin}/api/auth/providers"))
                .send()
                .await
                .map_err(unreachable)?;
            if !providers.status().is_success() {
                return Err(not_pingo());
            }
            VersionInfo::legacy()
        }
    };
    let app = VersionInfo::current(app.package_info().version.to_string());
    Ok(ServerCheck {
        compatibility: compatibility(&app, &server),
        origin,
        server,
        app,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_documents_are_read_and_html_fallbacks_are_not() {
        let info = VersionInfo::current("0.2.0");
        let json = serde_json::to_vec(&info).unwrap();
        assert_eq!(parse_version(true, &json), Some(info));
        assert_eq!(
            parse_version(true, b"<!doctype html><title>Pingo</title>"),
            None
        );
        assert_eq!(parse_version(false, &json), None);
    }
}
