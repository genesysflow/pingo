use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Mutex};
use url::Url;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    pub server_url: Option<String>,
    pub push_to_talk_shortcut: Option<String>,
}

pub struct DesktopState {
    pub preferences: Mutex<Preferences>,
    pub invites: Mutex<Vec<String>>,
}

pub fn server_origin(value: &str) -> Result<String, String> {
    let url =
        Url::parse(value.trim()).map_err(|_| "Enter a full http:// or https:// server URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Use a server origin without credentials, a path, query or fragment".into());
    }
    Ok(url.origin().ascii_serialization())
}

pub fn invite_path(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    let code = url.path().strip_prefix('/')?;
    (url.scheme() == "pingo"
        && url.host_str() == Some("invite")
        && !code.is_empty()
        && code.len() <= 256
        && code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && url.query().is_none()
        && url.fragment().is_none())
    .then(|| format!("/invite/{code}"))
}

impl Preferences {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let parent = path.parent().ok_or("Missing config directory")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let temporary = path.with_extension("tmp");
        std::fs::write(
            &temporary,
            serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(temporary, path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_urls_are_origins_and_do_not_carry_credentials() {
        assert_eq!(
            server_origin("https://pingo.example/").unwrap(),
            "https://pingo.example"
        );
        for value in [
            "file:///etc/passwd",
            "https://user:secret@example.com",
            "https://host/api",
            "https://host/?a=b",
        ] {
            assert!(server_origin(value).is_err(), "{value}");
        }
    }

    #[test]
    fn only_invite_deep_links_are_routable() {
        assert_eq!(
            invite_path("pingo://invite/abc-123"),
            Some("/invite/abc-123".into())
        );
        for value in [
            "https://invite/abc",
            "pingo://invite/",
            "pingo://invite/a/b",
            "pingo://invite/%2f",
            "pingo://invite/abc?q=x",
        ] {
            assert!(invite_path(value).is_none());
        }
    }

    #[test]
    fn preferences_survive_a_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        let preferences = Preferences {
            server_url: Some("http://localhost:8090".into()),
            push_to_talk_shortcut: None,
        };
        preferences.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).server_url, preferences.server_url);
    }
}
