pub mod calls;
mod config;

use config::{DesktopState, Preferences};
use tauri::{
    Emitter, Manager,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;

fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn accept_links(app: &tauri::AppHandle, urls: impl IntoIterator<Item = String>) {
    let paths: Vec<_> = urls
        .into_iter()
        .filter_map(|url| config::invite_path(&url))
        .collect();
    if paths.is_empty() {
        return;
    }
    app.state::<DesktopState>()
        .invites
        .lock()
        .unwrap()
        .extend(paths);
    let _ = app.emit("desktop-invite", ());
    show(app);
}

#[tauri::command]
fn desktop_config(state: tauri::State<'_, DesktopState>) -> Preferences {
    state.preferences.lock().unwrap().clone()
}

#[tauri::command]
fn desktop_save_config(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    mut preferences: Preferences,
) -> Result<Preferences, String> {
    preferences.server_url = preferences
        .server_url
        .as_deref()
        .map(config::server_origin)
        .transpose()?;
    let shortcut = preferences
        .push_to_talk_shortcut
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<Shortcut>().map_err(|e| e.to_string()))
        .transpose()?;
    let mut current = state.preferences.lock().unwrap();
    let old = current
        .push_to_talk_shortcut
        .as_deref()
        .and_then(|s| s.parse::<Shortcut>().ok());
    if shortcut != old {
        if let Some(shortcut) = shortcut {
            app.global_shortcut().register(shortcut).map_err(|e| {
                format!("Global shortcut unavailable: {e}. Use in-app push-to-talk.")
            })?;
        }
        if let Some(old) = old {
            let _ = app.global_shortcut().unregister(old);
        }
    }
    let path = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("preferences.json");
    if let Err(error) = preferences.save(&path) {
        if shortcut != old {
            if let Some(shortcut) = shortcut {
                let _ = app.global_shortcut().unregister(shortcut);
            }
            if let Some(old) = old {
                let _ = app.global_shortcut().register(old);
            }
        }
        return Err(error);
    }
    *current = preferences.clone();
    Ok(preferences)
}

#[tauri::command]
fn desktop_invites(state: tauri::State<'_, DesktopState>) -> Vec<String> {
    std::mem::take(&mut *state.invites.lock().unwrap())
}

#[tauri::command]
fn desktop_notify(app: tauri::AppHandle, title: String, body: String) -> Result<(), String> {
    if title.len() > 256 || body.len() > 4096 {
        return Err("Notification is too long".into());
    }
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _| { show(app); accept_links(app, args); }))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().with_handler(|app, _, event| {
            let _ = app.state::<calls::CallManager>().ptt(event.state == ShortcutState::Pressed);
            let _ = app.emit("desktop-ptt", event.state == ShortcutState::Pressed);
        }).build())
        .invoke_handler(tauri::generate_handler![desktop_config, desktop_save_config, desktop_invites, desktop_notify, call_join, call_leave, call_configure, call_data, call_ptt, call_devices, call_loopback])
        .setup(|app| {
            app.manage(calls::CallManager::default());
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut last = Vec::new();
                loop {
                    if let Ok(devices) = calls::devices() && devices != last { let _ = handle.emit("native-devices", &devices); last = devices; }
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
            });
            let preferences = Preferences::load(&app.path().app_config_dir()?.join("preferences.json"));
            if let Some(shortcut) = preferences.push_to_talk_shortcut.as_deref() && let Err(error) = app.global_shortcut().register(shortcut) { tracing::warn!(%error, "global push-to-talk unavailable; use in-app shortcut"); }
            app.manage(DesktopState { preferences: std::sync::Mutex::new(preferences), invites: std::sync::Mutex::new(Vec::new()) });
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| accept_links(&handle, event.urls().into_iter().map(|u| u.to_string())));
            if let Some(urls) = app.deep_link().get_current()? { accept_links(app.handle(), urls.into_iter().map(|u| u.to_string())); }
            #[cfg(target_os = "linux")]
            if std::env::var_os("WAYLAND_DISPLAY").is_some() { tracing::info!("Wayland: global shortcuts may require XWayland; in-app push-to-talk remains available"); }
            let open = MenuItem::with_id(app, "open", "Open Pingo", true, None::<&str>)?;
            let mute = MenuItem::with_id(app, "mute", "Mute / unmute", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Pingo", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &mute, &quit])?;
            TrayIconBuilder::new().icon(app.default_window_icon().unwrap().clone()).menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show(app),
                    "mute" => { if let Ok(settings) = app.state::<calls::CallManager>().toggle_mute() { let _ = app.emit("desktop-mute", settings.muted); } },
                    "quit" => app.exit(0),
                    _ => {}
                }).build(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Pingo desktop failed to start");
}

fn event_sink(app: tauri::AppHandle) -> calls::EventSink {
    std::sync::Arc::new(move |event| {
        let _ = app.emit("native-call", event);
    })
}
#[tauri::command]
fn call_join(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    manager: tauri::State<'_, calls::CallManager>,
    session_id: String,
    url: String,
    token: String,
    settings: calls::Settings,
) -> Result<(), String> {
    let origin = state
        .preferences
        .lock()
        .unwrap()
        .server_url
        .clone()
        .ok_or("Choose a server first")?;
    manager
        .join(
            calls::JoinRequest {
                id: session_id,
                origin: &origin,
                url: &url,
                token,
                settings,
            },
            event_sink(app),
            true,
        )
        .map_err(|e| e.to_string())
}
#[tauri::command]
fn call_leave(manager: tauri::State<'_, calls::CallManager>, session_id: String) {
    manager.leave(Some(&session_id));
}
#[tauri::command]
fn call_configure(
    manager: tauri::State<'_, calls::CallManager>,
    session_id: String,
    settings: calls::Settings,
) -> Result<(), String> {
    manager
        .configure(&session_id, settings)
        .map_err(|e| e.to_string())
}
#[tauri::command]
fn call_data(
    manager: tauri::State<'_, calls::CallManager>,
    session_id: String,
    data: pingo_proto::DataMsg,
) -> Result<(), String> {
    manager.send(&session_id, data).map_err(|e| e.to_string())
}
#[tauri::command]
fn call_ptt(manager: tauri::State<'_, calls::CallManager>, pressed: bool) -> Result<(), String> {
    manager.ptt(pressed).map_err(|e| e.to_string())
}
#[tauri::command]
fn call_devices() -> Result<Vec<calls::Device>, String> {
    calls::devices()
}
#[tauri::command]
fn call_loopback(
    app: tauri::AppHandle,
    manager: tauri::State<'_, calls::CallManager>,
    session_id: String,
    settings: calls::Settings,
) -> Result<(), String> {
    manager
        .loopback(session_id, settings, event_sink(app))
        .map_err(|e| e.to_string())
}
