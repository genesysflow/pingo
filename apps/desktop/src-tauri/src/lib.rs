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
            let _ = app.emit("desktop-ptt", event.state == ShortcutState::Pressed);
        }).build())
        .invoke_handler(tauri::generate_handler![desktop_config, desktop_save_config, desktop_invites, desktop_notify])
        .setup(|app| {
            let preferences = Preferences::load(&app.path().app_config_dir()?.join("preferences.json"));
            if let Some(shortcut) = preferences.push_to_talk_shortcut.as_deref() {
                if let Err(error) = app.global_shortcut().register(shortcut) { tracing::warn!(%error, "global push-to-talk unavailable; use in-app shortcut"); }
            }
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
                    "mute" => { let _ = app.emit("desktop-mute", ()); },
                    "quit" => app.exit(0),
                    _ => {}
                }).build(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Pingo desktop failed to start");
}
