//! Self-updates through the Tauri updater. Releases publish a manifest and bundles signed with the key whose
//! public half is in `tauri.conf.json`; the plugin refuses anything else. Updates download in the background
//! and install only when the user restarts, never during a call.
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, Updater, UpdaterExt};

use crate::calls::CallManager;

const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// Lets a development build, or a tester, use another manifest. Signatures are still verified.
const ENDPOINT_ENV: &str = "PINGO_UPDATE_ENDPOINT";

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum UpdateStatus {
    /// No release feed: development builds, unless [`ENDPOINT_ENV`] is set.
    Disabled,
    #[default]
    Idle,
    Checking,
    UpToDate,
    Downloading {
        version: String,
    },
    Ready {
        version: String,
        notes: Option<String>,
    },
    Failed {
        message: String,
    },
}

#[derive(Default)]
pub struct Updates {
    status: Mutex<UpdateStatus>,
    ready: Mutex<Option<(Update, Vec<u8>)>>,
    /// One check or download at a time; the periodic check and the Settings button can overlap.
    busy: tokio::sync::Mutex<()>,
}

impl Updates {
    fn status(&self) -> UpdateStatus {
        self.status.lock().unwrap().clone()
    }
}

fn set(app: &AppHandle, status: UpdateStatus) -> UpdateStatus {
    *app.state::<Updates>().status.lock().unwrap() = status.clone();
    let _ = app.emit("desktop-update", &status);
    status
}

fn updater(app: &AppHandle) -> Option<Result<Updater, String>> {
    let endpoint = std::env::var(ENDPOINT_ENV).ok();
    if cfg!(debug_assertions) && endpoint.is_none() {
        return None;
    }
    let mut builder = app.updater_builder();
    if let Some(endpoint) = endpoint {
        let url = match endpoint.parse() {
            Ok(url) => url,
            Err(e) => return Some(Err(format!("{ENDPOINT_ENV}: {e}"))),
        };
        builder = match builder.endpoints(vec![url]) {
            Ok(builder) => builder,
            Err(e) => return Some(Err(e.to_string())),
        };
    }
    match builder.build() {
        Err(tauri_plugin_updater::Error::EmptyEndpoints) => None,
        result => Some(result.map_err(|e| e.to_string())),
    }
}

/// Looks for a release and downloads it. Returns the resulting status, which is also emitted.
pub async fn check(app: &AppHandle) -> UpdateStatus {
    let updates = app.state::<Updates>();
    let Ok(_busy) = updates.busy.try_lock() else {
        return updates.status();
    };
    if let status @ UpdateStatus::Ready { .. } = updates.status() {
        return status;
    }
    let updater = match updater(app) {
        None => return set(app, UpdateStatus::Disabled),
        Some(Err(message)) => return set(app, UpdateStatus::Failed { message }),
        Some(Ok(updater)) => updater,
    };
    set(app, UpdateStatus::Checking);
    let update = match updater.check().await {
        Ok(Some(update)) => update,
        Ok(None) => return set(app, UpdateStatus::UpToDate),
        Err(e) => {
            return set(
                app,
                UpdateStatus::Failed {
                    message: e.to_string(),
                },
            );
        }
    };
    let version = update.version.clone();
    set(
        app,
        UpdateStatus::Downloading {
            version: version.clone(),
        },
    );
    match update.download(|_, _| {}, || {}).await {
        Ok(bytes) => {
            let notes = update.body.clone();
            *updates.ready.lock().unwrap() = Some((update, bytes));
            tracing::info!(%version, "update downloaded");
            set(app, UpdateStatus::Ready { version, notes })
        }
        Err(e) => set(
            app,
            UpdateStatus::Failed {
                message: e.to_string(),
            },
        ),
    }
}

/// Checks shortly after launch, then periodically.
pub fn spawn_checks(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            if let UpdateStatus::Failed { message } = check(&app).await {
                tracing::warn!(%message, "update check failed");
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

#[tauri::command]
pub fn desktop_update_status(updates: tauri::State<'_, Updates>) -> UpdateStatus {
    updates.status()
}

#[tauri::command]
pub async fn desktop_update_check(app: AppHandle) -> UpdateStatus {
    check(&app).await
}

#[tauri::command]
pub fn desktop_update_install(
    app: AppHandle,
    updates: tauri::State<'_, Updates>,
    calls: tauri::State<'_, CallManager>,
) -> Result<(), String> {
    if calls.in_call() {
        return Err("Leave the call before restarting Pingo.".into());
    }
    let (update, bytes) = updates
        .ready
        .lock()
        .unwrap()
        .take()
        .ok_or("No update is ready")?;
    if let Err(e) = update.install(&bytes) {
        let message = e.to_string();
        set(
            &app,
            UpdateStatus::Failed {
                message: message.clone(),
            },
        );
        return Err(message);
    }
    app.restart()
}

#[tauri::command]
pub fn desktop_app_info(app: AppHandle) -> pingo_proto::VersionInfo {
    pingo_proto::VersionInfo::current(app.package_info().version.to_string())
}
